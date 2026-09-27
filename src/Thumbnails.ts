import Gdk from "gi://Gdk?version=4.0"
import GdkPixbuf from "gi://GdkPixbuf"
import Gio from "gi://Gio"
import GLib from "gi://GLib"
import { APP_NAME, basename, cached, OLD_APP_NAME } from "./util"

// Decoded size of cached thumbnails (the cache keeps these dimensions so
// existing thumbnails stay valid).
const THUMBNAIL_WIDTH = 440
const THUMBNAIL_HEIGHT = 320
const THUMBNAIL_CONCURRENCY = 4

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
  const existing = textures.get(file)
  if (existing) return Promise.resolve(existing)

  const inFlight = loads.get(file)
  if (inFlight) return inFlight

  const promise = (async () => {
    const path = cachePath(file, mtime)

    if (GLib.file_test(path, GLib.FileTest.EXISTS)) {
      try {
        return Gdk.Texture.new_for_pixbuf(await decodeImage(path, false))
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
      textures.set(file, texture)
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
