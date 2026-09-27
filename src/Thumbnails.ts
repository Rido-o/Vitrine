import Gdk from "gi://Gdk?version=4.0"
import GdkPixbuf from "gi://GdkPixbuf"
import Gio from "gi://Gio"
import GLib from "gi://GLib"
import System from "system"
import { APP_NAME, basename, cached, OLD_APP_NAME } from "./util"

// Decoded size of cached thumbnails (the cache keeps these dimensions so
// existing thumbnails stay valid).
const THUMBNAIL_WIDTH = 440
const THUMBNAIL_HEIGHT = 320
const THUMBNAIL_CONCURRENCY = 4

// Cache files are named after the image's path and mtime, so edited, moved or
// deleted images leave orphans. Files are touched when used (at most daily)
// and pruned once unused for PRUNE_AFTER_DAYS.
const PRUNE_AFTER_DAYS = 90
const TOUCH_AFTER_SECONDS = 24 * 60 * 60
const CACHE_FILE = /^[0-9a-f]{32}\.jpg$/

// Decoded thumbnails kept in memory (~0.5 MB each), least recently used
// dropped first; a dropped tile that scrolls back into view reloads from disk.
const MAX_TEXTURES = 300
// Dropped textures (and each decode's pixbuf) are only freed when GJS collects
// their wrappers, and its GC doesn't see their native memory, so without a
// nudge memory kept growing past the cap (526 MB vs 271 MB after 1,000
// thumbnails). Collect after this many evictions (~50 MB).
const GC_AFTER_EVICTIONS = 100

const THUMBNAIL_CACHE = GLib.build_filenamev([
  GLib.get_user_cache_dir(),
  APP_NAME,
  "thumbnails",
])
// Vitrine was called shard-view; its cache uses the same naming, so move it
// over once instead of regenerating it.
const OLD_THUMBNAIL_CACHE = GLib.build_filenamev([
  GLib.get_user_cache_dir(),
  OLD_APP_NAME,
  "thumbnails",
])

if (
  !GLib.file_test(THUMBNAIL_CACHE, GLib.FileTest.EXISTS) &&
  GLib.file_test(OLD_THUMBNAIL_CACHE, GLib.FileTest.IS_DIR)
) {
  GLib.mkdir_with_parents(GLib.path_get_dirname(THUMBNAIL_CACHE), 0o755)
  if (GLib.rename(OLD_THUMBNAIL_CACHE, THUMBNAIL_CACHE) !== 0) {
    console.error("Could not move the old thumbnail cache; starting fresh")
  }
}
GLib.mkdir_with_parents(THUMBNAIL_CACHE, 0o755)

const textures = new Map<string, Gdk.Texture>()
const loads = new Map<string, Promise<Gdk.Texture>>()
const imageInfo = new Map<string, { filename: string; resolution: string }>()
const touched = new Set<string>()

// Map keeps insertion order, so re-inserting on use keeps the least recently
// used texture first.
function getTexture(file: string) {
  const texture = textures.get(file)
  if (texture) {
    textures.delete(file)
    textures.set(file, texture)
  }
  return texture
}

let evictions = 0

function rememberTexture(file: string, texture: Gdk.Texture) {
  textures.delete(file)
  textures.set(file, texture)
  while (textures.size > MAX_TEXTURES) {
    textures.delete(textures.keys().next().value!)
    if (++evictions === GC_AFTER_EVICTIONS) {
      evictions = 0
      GLib.idle_add(GLib.PRIORITY_LOW, () => {
        System.gc()
        return GLib.SOURCE_REMOVE
      })
    }
  }
}

// For diagnostics and headless tests.
export function texturesInMemory() {
  return textures.size
}

function nowSeconds() {
  return Math.floor(GLib.get_real_time() / 1_000_000)
}

// Marks a cache file as used so pruning keeps it.
function touchIfStale(path: string) {
  if (touched.has(path)) return
  touched.add(path)
  try {
    const file = Gio.File.new_for_path(path)
    const info = file.query_info(
      "time::modified",
      Gio.FileQueryInfoFlags.NONE,
      null,
    )
    const modified = info.get_modification_date_time()?.to_unix() ?? 0
    if (nowSeconds() - modified < TOUCH_AFTER_SECONDS) return
    file.set_attribute_uint64(
      "time::modified",
      nowSeconds(),
      Gio.FileQueryInfoFlags.NONE,
      null,
    )
  } catch (error) {
    console.error(`Could not touch cached thumbnail ${path}:`, error)
  }
}

let activeLoads = 0
const queue: Array<() => void> = []

function withConcurrencyLimit<T>(task: () => Promise<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    const run = () => {
      activeLoads++
      task()
        .then(resolve, reject)
        .finally(() => {
          activeLoads--
          queue.shift()?.()
        })
    }
    if (activeLoads < THUMBNAIL_CONCURRENCY) run()
    else queue.push(run)
  })
}

function cachePath(file: string, mtime: number) {
  const hash = GLib.compute_checksum_for_string(
    GLib.ChecksumType.MD5,
    `${file}:${mtime}`,
    -1,
  )
  return GLib.build_filenamev([THUMBNAIL_CACHE, `${hash}.jpg`])
}

function decodeImage(path: string, scale: boolean): Promise<GdkPixbuf.Pixbuf> {
  return new Promise((resolve, reject) => {
    Gio.File.new_for_path(path).read_async(
      GLib.PRIORITY_LOW,
      null,
      (source, result) => {
        let stream: Gio.InputStream
        try {
          stream = (source as Gio.File).read_finish(result)
        } catch (error) {
          reject(error)
          return
        }
        const onDone = (_source: unknown, res: Gio.AsyncResult) => {
          try {
            const pixbuf = GdkPixbuf.Pixbuf.new_from_stream_finish(res)
            stream.close(null)
            if (!pixbuf) throw new Error("Could not decode image")
            resolve(pixbuf)
          } catch (error) {
            stream.close(null)
            reject(error)
          }
        }
        if (scale) {
          GdkPixbuf.Pixbuf.new_from_stream_at_scale_async(
            stream,
            THUMBNAIL_WIDTH,
            THUMBNAIL_HEIGHT,
            true,
            null,
            onDone,
          )
        } else {
          GdkPixbuf.Pixbuf.new_from_stream_async(stream, null, onDone)
        }
      },
    )
  })
}

export function loadThumbnail(file: string, mtime: number) {
  const existing = getTexture(file)
  if (existing) return Promise.resolve(existing)

  const inFlight = loads.get(file)
  if (inFlight) return inFlight

  const promise = (async () => {
    const path = cachePath(file, mtime)

    if (GLib.file_test(path, GLib.FileTest.EXISTS)) {
      try {
        const texture = Gdk.Texture.new_for_pixbuf(
          await decodeImage(path, false),
        )
        touchIfStale(path)
        return texture
      } catch (error) {
        console.error(`Could not load cached thumbnail ${file}:`, error)
      }
    }

    return withConcurrencyLimit(async () => {
      const pixbuf = await decodeImage(file, true)
      try {
        pixbuf.savev(path, "jpeg", ["quality"], ["85"])
      } catch (error) {
        console.error(`Could not cache thumbnail ${file}:`, error)
      }
      return Gdk.Texture.new_for_pixbuf(pixbuf)
    })
  })()
    .then((texture) => {
      rememberTexture(file, texture)
      return texture
    })
    .catch((error) => {
      console.error(`Could not load thumbnail ${file}:`, error)
      throw error
    })
    .finally(() => loads.delete(file))

  loads.set(file, promise)
  return promise
}

export function getImageInfo(file: string) {
  return cached(imageInfo, file, () => {
    const [, width, height] = GdkPixbuf.Pixbuf.get_file_info(file)
    return { filename: basename(file), resolution: `${width} × ${height}` }
  })
}

export function evictThumbnail(file: string) {
  textures.delete(file)
  imageInfo.delete(file)
}

// Deletes cache files unused for PRUNE_AFTER_DAYS; resolves to how many.
export async function pruneThumbnailCache() {
  const cutoff = nowSeconds() - PRUNE_AFTER_DAYS * 24 * 60 * 60
  const directory = Gio.File.new_for_path(THUMBNAIL_CACHE)
  let pruned = 0
  try {
    const enumerator = await new Promise<Gio.FileEnumerator>(
      (resolve, reject) =>
        directory.enumerate_children_async(
          "standard::name,time::modified",
          Gio.FileQueryInfoFlags.NONE,
          GLib.PRIORITY_LOW,
          null,
          (_source, result) => {
            try {
              resolve(directory.enumerate_children_finish(result))
            } catch (error) {
              reject(error)
            }
          },
        ),
    )
    let infos: Gio.FileInfo[]
    while (
      (infos = await new Promise<Gio.FileInfo[]>((resolve, reject) =>
        enumerator.next_files_async(
          200,
          GLib.PRIORITY_LOW,
          null,
          (_source, result) => {
            try {
              resolve(enumerator.next_files_finish(result))
            } catch (error) {
              reject(error)
            }
          },
        ),
      )).length > 0
    ) {
      for (const info of infos) {
        const name = info.get_name()
        const modified = info.get_modification_date_time()?.to_unix() ?? 0
        if (!CACHE_FILE.test(name) || modified >= cutoff) continue
        try {
          directory.get_child(name).delete(null)
          pruned++
        } catch (error) {
          // Another Vitrine process may be pruning at the same time.
          if (
            !(error instanceof GLib.Error) ||
            !error.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.NOT_FOUND)
          )
            console.error(`Could not prune ${name}:`, error)
        }
      }
    }
    enumerator.close(null)
  } catch (error) {
    console.error("Could not prune the thumbnail cache:", error)
  }
  return pruned
}
