import Gdk from "gi://Gdk?version=4.0"
import GdkPixbuf from "gi://GdkPixbuf"
import Gio from "gi://Gio"
import GLib from "gi://GLib"
import System from "system"
import { decodeImage } from "./decode"
import { APP_NAME, basename, cached, OLD_APP_NAME } from "./util"

// Decoded size of cached thumbnails (changing it means a new cache folder).
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

// "-2": thumbnails since EXIF orientation is applied. Older caches hold
// unrotated thumbnails that can't be told apart, so they're deleted instead.
const THUMBNAIL_CACHE = GLib.build_filenamev([
  GLib.get_user_cache_dir(),
  APP_NAME,
  "thumbnails-2",
])
const OLD_THUMBNAIL_CACHES = [
  GLib.build_filenamev([GLib.get_user_cache_dir(), APP_NAME, "thumbnails"]),
  GLib.build_filenamev([GLib.get_user_cache_dir(), OLD_APP_NAME]),
]
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

export function loadThumbnail(file: string, mtime: number) {
  const existing = getTexture(file)
  if (existing) return Promise.resolve(existing)

  const inFlight = loads.get(file)
  if (inFlight) return inFlight

  const promise = (async () => {
    const path = cachePath(file, mtime)

    if (GLib.file_test(path, GLib.FileTest.EXISTS)) {
      try {
        const texture = Gdk.Texture.new_for_pixbuf(await decodeImage(path))
        touchIfStale(path)
        return texture
      } catch (error) {
        console.error(`Could not load cached thumbnail ${file}:`, error)
      }
    }

    return withConcurrencyLimit(async () => {
      const pixbuf = await decodeImage(file, {
        width: THUMBNAIL_WIDTH,
        height: THUMBNAIL_HEIGHT,
      })
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

// The thumbnail if it's already in memory; never loads.
export function peekThumbnail(file: string) {
  return textures.get(file) ?? null
}

export function evictThumbnail(file: string) {
  textures.delete(file)
  imageInfo.delete(file)
}

// Calls `visit` for each entry in `directory`, in batches at low priority so a
// large cache doesn't hold up the window.
async function forEachChild(
  directory: Gio.File,
  visit: (info: Gio.FileInfo) => void,
) {
  const enumerator = await new Promise<Gio.FileEnumerator>((resolve, reject) =>
    directory.enumerate_children_async(
      "standard::name,standard::type,time::modified",
      Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS,
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
    infos.forEach(visit)
  }
  enumerator.close(null)
}

// Deletes `file`; false if it failed. Another Vitrine process may be deleting
// the same files, so one that's already gone isn't an error.
function remove(file: Gio.File) {
  try {
    file.delete(null)
    return true
  } catch (error) {
    if (
      !(error instanceof GLib.Error) ||
      !error.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.NOT_FOUND)
    )
      console.error(`Could not delete ${file.get_path()}:`, error)
    return false
  }
}

// Deletes cache files unused for PRUNE_AFTER_DAYS; resolves to how many.
export async function pruneThumbnailCache() {
  const cutoff = nowSeconds() - PRUNE_AFTER_DAYS * 24 * 60 * 60
  const directory = Gio.File.new_for_path(THUMBNAIL_CACHE)
  let pruned = 0
  try {
    await forEachChild(directory, (info) => {
      const name = info.get_name()
      const modified = info.get_modification_date_time()?.to_unix() ?? 0
      if (CACHE_FILE.test(name) && modified < cutoff) {
        if (remove(directory.get_child(name))) pruned++
      }
    })
  } catch (error) {
    console.error("Could not prune the thumbnail cache:", error)
  }
  return pruned
}

async function removeTree(directory: Gio.File): Promise<number> {
  const subdirectories: Gio.File[] = []
  let removed = 0
  await forEachChild(directory, (info) => {
    const child = directory.get_child(info.get_name())
    if (info.get_file_type() === Gio.FileType.DIRECTORY)
      subdirectories.push(child)
    else if (remove(child)) removed++
  })
  for (const subdirectory of subdirectories) {
    removed += await removeTree(subdirectory)
  }
  remove(directory)
  return removed
}

// Deletes the caches from before rotation was applied (see THUMBNAIL_CACHE);
// resolves to how many files that removed.
export async function removeOldThumbnailCaches() {
  let removed = 0
  for (const path of OLD_THUMBNAIL_CACHES) {
    if (!GLib.file_test(path, GLib.FileTest.IS_DIR)) continue
    try {
      removed += await removeTree(Gio.File.new_for_path(path))
    } catch (error) {
      console.error(`Could not remove ${path}:`, error)
    }
  }
  return removed
}
