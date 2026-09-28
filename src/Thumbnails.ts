import Gdk from "gi://Gdk?version=4.0"
import GdkPixbuf from "gi://GdkPixbuf"
import Gio from "gi://Gio"
import GLib from "gi://GLib"
import { decodeImage } from "./decode"
import {
  APP_NAME,
  basename,
  cached,
  listDirectory,
  OLD_APP_NAME,
  requestGc,
} from "./util"

// Decoded size of cached thumbnails (changing it means a new cache folder).
const THUMBNAIL_WIDTH = 440
const THUMBNAIL_HEIGHT = 320
const THUMBNAIL_CONCURRENCY = 4
// Background generation uses fewer, leaving room for tiles scrolled to, and
// checks this many images for a cached thumbnail per pass.
const BACKGROUND_CONCURRENCY = 2
const BACKGROUND_CHECKS = 100

// Cache files are named after the image's path and mtime, so edited, moved or
// deleted images leave orphans. Files are touched when used (at most daily)
// and pruned once unused for PRUNE_AFTER_DAYS.
const PRUNE_AFTER_DAYS = 90
const TOUCH_AFTER_SECONDS = 24 * 60 * 60
const CACHE_FILE = /^[0-9a-f]{32}\.(jpg|png)$/

// Decoded thumbnails kept for tiles no longer shown (~0.5 MB each), least
// recently used dropped first; a dropped tile that scrolls back into view
// reloads from disk. Shown tiles' textures are never dropped: the grid keeps
// ~390 tiles bound (rows around the viewport), and a cap of 300 on all of them
// had off-screen tiles evicting visible ones (the full-screen view then opened
// without its placeholder).
const MAX_UNWANTED = 100
// Dropped textures and each decode's native memory are only freed when GJS
// collects their wrappers, and its GC doesn't see that memory, so it's nudged.
// Without it, memory kept growing past the cap (526 MB vs 271 MB after 1,000
// thumbnails), and generating thumbnails of large images kept ~8 MB each
// alive (a 2.9 GB peak for 348 screenshots vs ~0.33 GB). Collect after this
// many evictions (~50 MB) or generated thumbnails.
const GC_AFTER_EVICTIONS = 100
const GC_AFTER_GENERATED = 5

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

// In memory by path and mtime, so a load started before a file changed can't
// stand in for the new version.
const textures = new Map<string, Gdk.Texture>()
const loads = new Map<string, Promise<Gdk.Texture>>()
const imageInfo = new Map<string, { filename: string; resolution: string }>()
const touched = new Set<string>()

function memoryKey(file: string, mtime: number) {
  return `${file}\n${mtime}`
}

// Map keeps insertion order, so re-inserting on use keeps the least recently
// used texture first.
function getTexture(key: string) {
  const texture = textures.get(key)
  if (texture) {
    textures.delete(key)
    textures.set(key, texture)
  }
  return texture
}

let evictions = 0
let generated = 0
function rememberTexture(key: string, texture: Gdk.Texture) {
  textures.delete(key)
  textures.set(key, texture)
  let unwanted = 0
  for (const kept of textures.keys()) if (!wanted.has(kept)) unwanted++
  for (const kept of textures.keys()) {
    if (unwanted <= MAX_UNWANTED) break
    if (wanted.has(kept)) continue
    textures.delete(kept)
    unwanted--
    if (++evictions === GC_AFTER_EVICTIONS) {
      evictions = 0
      requestGc()
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

// Cache files are named after a hash of the path and mtime. Formats that can
// be transparent were once saved as JPEG, losing it (transparent areas came
// out black); a suffix gives their thumbnails new names, so those old ones go
// unused and get pruned.
function cacheBase(file: string, mtime: number) {
  const canBeTransparent = !/\.jpe?g$/i.test(file)
  const hash = GLib.compute_checksum_for_string(
    GLib.ChecksumType.MD5,
    canBeTransparent ? `${file}:${mtime}:alpha` : `${file}:${mtime}`,
    -1,
  )
  return GLib.build_filenamev([THUMBNAIL_CACHE, `${hash}`])
}

// The cached thumbnail's path, if there is one: a JPEG, or a PNG when the
// image has transparent pixels.
function cachedFile(file: string, mtime: number) {
  const base = cacheBase(file, mtime)
  for (const path of [`${base}.jpg`, `${base}.png`]) {
    if (GLib.file_test(path, GLib.FileTest.EXISTS)) return path
  }
  return null
}

function hasTransparency(pixbuf: GdkPixbuf.Pixbuf) {
  if (!pixbuf.get_has_alpha()) return false
  const pixels = pixbuf.get_pixels()
  const stride = pixbuf.get_rowstride()
  const channels = pixbuf.get_n_channels()
  for (let y = 0; y < pixbuf.get_height(); y++) {
    const row = y * stride
    for (let x = 0; x < pixbuf.get_width(); x++) {
      if (pixels[row + x * channels + 3] < 255) return true
    }
  }
  return false
}

// Decodes `file` down to thumbnail size and saves it to the cache.
async function generate(file: string, mtime: number) {
  const pixbuf = await decodeImage(file, {
    width: THUMBNAIL_WIDTH,
    height: THUMBNAIL_HEIGHT,
  }).finally(() => {
    if (++generated === GC_AFTER_GENERATED) {
      generated = 0
      requestGc()
    }
  })
  const base = cacheBase(file, mtime)
  try {
    if (hasTransparency(pixbuf)) pixbuf.savev(`${base}.png`, "png", [], [])
    else pixbuf.savev(`${base}.jpg`, "jpeg", ["quality"], ["85"])
  } catch (error) {
    console.error(`Could not cache thumbnail ${file}:`, error)
  }
  return pixbuf
}

// Rejection of a thumbnail no tile wanted any more by the time its turn came.
export class Skipped extends Error {}

// Generating (decoding a full image) is the slow part, so it's queued, at most
// THUMBNAIL_CONCURRENCY at a time. Tiles' requests go newest first (the ones
// just scrolled to), and one no tile wants any longer is skipped, so scrolling
// fast through a big folder doesn't build a backlog of tiles already passed.
// A folder's other missing thumbnails are generated in the background, only
// while no tile is waiting.
type Job = { run: () => Promise<unknown>; skip: () => void; key: string }
const requested: Job[] = []
let active = 0
// How many bound tiles want each thumbnail (loadThumbnail adds one,
// releaseThumbnail takes it back).
const wanted = new Map<string, number>()
let background: Array<[string, number]> = []
let backgroundNext = 0

function pump() {
  while (active < THUMBNAIL_CONCURRENCY) {
    const job =
      requested.pop() ??
      (active < BACKGROUND_CONCURRENCY ? nextBackgroundJob() : undefined)
    if (!job) return
    if (job.key && !wanted.has(job.key)) {
      job.skip()
      continue
    }
    active++
    job.run().finally(() => {
      active--
      pump()
    })
  }
}

// The next background thumbnail that isn't cached, loading or in memory. Looks
// at a limited number per call (each is a stat), continuing when idle, so a
// big folder that's already cached doesn't block the window.
function nextBackgroundJob(): Job | undefined {
  for (let checked = 0; checked < BACKGROUND_CHECKS; checked++) {
    const item = background[backgroundNext++]
    if (!item) return undefined
    const [file, mtime] = item
    const key = memoryKey(file, mtime)
    if (loads.has(key) || textures.has(key) || cachedFile(file, mtime)) {
      continue
    }
    return {
      key: "",
      skip: () => {},
      run: () =>
        generate(file, mtime).catch((error) =>
          console.error(`Could not generate thumbnail ${file}:`, error),
        ),
    }
  }
  GLib.idle_add(GLib.PRIORITY_LOW, () => {
    pump()
    return GLib.SOURCE_REMOVE
  })
  return undefined
}

// Generates the missing thumbnails of `images` ([path, mtime]) in the
// background, replacing any earlier list (one window per process is usual).
export function generateInBackground(images: Array<[string, number]>) {
  background = images
  backgroundNext = 0
  pump()
}

export function stopBackgroundGeneration(images: Array<[string, number]>) {
  if (background !== images) return
  background = []
  backgroundNext = 0
}

function queueGeneration(
  key: string,
  generateTexture: () => Promise<Gdk.Texture>,
) {
  return new Promise<Gdk.Texture>((resolve, reject) => {
    requested.push({
      key,
      skip: () => reject(new Skipped()),
      run: () => generateTexture().then(resolve, reject),
    })
    pump()
  })
}

// Loads a tile's thumbnail from memory, the cache or the image; call
// releaseThumbnail when the tile no longer shows it. Rejects with Skipped if
// it's released before its turn to be generated.
export function loadThumbnail(file: string, mtime: number) {
  const key = memoryKey(file, mtime)
  wanted.set(key, (wanted.get(key) ?? 0) + 1)
  const existing = getTexture(key)
  if (existing) return Promise.resolve(existing)

  const inFlight = loads.get(key)
  if (inFlight) return inFlight

  const promise = (async () => {
    const path = cachedFile(file, mtime)
    if (path) {
      try {
        const texture = Gdk.Texture.new_for_pixbuf(await decodeImage(path))
        touchIfStale(path)
        return texture
      } catch (error) {
        console.error(`Could not load cached thumbnail ${file}:`, error)
      }
    }
    return queueGeneration(key, async () =>
      Gdk.Texture.new_for_pixbuf(await generate(file, mtime)),
    )
  })()
    .then((texture) => {
      rememberTexture(key, texture)
      return texture
    })
    .catch((error) => {
      if (!(error instanceof Skipped)) {
        console.error(`Could not load thumbnail ${file}:`, error)
      }
      throw error
    })
    .finally(() => loads.delete(key))

  loads.set(key, promise)
  return promise
}

export function releaseThumbnail(file: string, mtime: number) {
  const key = memoryKey(file, mtime)
  const count = (wanted.get(key) ?? 0) - 1
  if (count > 0) wanted.set(key, count)
  else wanted.delete(key)
}

export function getImageInfo(file: string) {
  return cached(imageInfo, file, () => {
    const [, width, height] = GdkPixbuf.Pixbuf.get_file_info(file)
    return { filename: basename(file), resolution: `${width} × ${height}` }
  })
}

// The thumbnail if it's already in memory; never loads.
export function peekThumbnail(file: string, mtime: number) {
  return textures.get(memoryKey(file, mtime)) ?? null
}

// Drops every version of `file`.
export function evictThumbnail(file: string) {
  const prefix = memoryKey(file, 0).slice(0, -1)
  for (const key of textures.keys()) {
    if (key.startsWith(prefix)) textures.delete(key)
  }
  imageInfo.delete(file)
}

// Calls `visit` for each entry in `directory`, in batches at low priority so a
// large cache doesn't hold up the window.
function forEachChild(
  directory: Gio.File,
  visit: (info: Gio.FileInfo) => void,
) {
  return listDirectory(
    directory,
    "standard::name,standard::type,time::modified",
    (infos) => infos.forEach(visit),
    {
      flags: Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS,
      priority: GLib.PRIORITY_LOW,
    },
  )
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
