import Gdk from "gi://Gdk?version=4.0"
import Gio from "gi://Gio"
import GdkPixbuf from "gi://GdkPixbuf"
import { copyToTexture, decodeAnimation, decodeImage } from "./decode"
import { requestGc } from "./util"

// At most this many full-size decodes run at once; the rest wait in a queue,
// so the image being shown never waits behind preloads.
const MAX_DECODES = 2

// `animation` is set for images with more than one frame (GIFs); `texture` is
// the first frame.
export type DecodedImage = {
  texture: Gdk.Texture
  animation: GdkPixbuf.PixbufAnimation | null
}

// Rejection of an entry dropped before its image was needed.
export class Cancelled extends Error {}

type Entry = {
  promise: Promise<DecodedImage>
  start: () => void
  // Stops a running decode, or settles a queued one that never started.
  drop: () => void
  started: boolean
}

// Full-size images for the full-screen view: the one shown and its
// neighbours, decoded in the background so ←/→ are instant. `keep` drops
// everything else, cancelling decodes still running and removing queued ones
// before they start, so holding an arrow key doesn't build a backlog of
// images already passed. Full-size textures are large (33 MB for 4K) and, like
// thumbnails, only freed when GJS collects them, so dropping one nudges the GC.
function isAnimated(path: string) {
  return path.toLowerCase().endsWith(".gif")
}

async function decode(
  path: string,
  cancellable: Gio.Cancellable,
): Promise<DecodedImage> {
  if (!isAnimated(path)) {
    const pixbuf = await decodeImage(path, undefined, cancellable)
    return { texture: Gdk.Texture.new_for_pixbuf(pixbuf), animation: null }
  }
  const animation = await decodeAnimation(path, cancellable)
  return {
    texture: copyToTexture(animation.get_static_image()),
    animation: animation.is_static_image() ? null : animation,
  }
}

export default class ImageCache {
  private entries = new Map<string, Entry>()
  private queue: string[] = []
  private active = 0

  private create(path: string): Entry {
    const cancellable = new Gio.Cancellable()
    let resolve!: (image: DecodedImage) => void
    let reject!: (error: unknown) => void
    const promise = new Promise<DecodedImage>((res, rej) => {
      resolve = res
      reject = rej
    })
    const entry: Entry = {
      promise,
      started: false,
      drop: () => {
        if (entry.started) cancellable.cancel()
        else reject(new Cancelled())
      },
      start: () => {
        entry.started = true
        this.active++
        decode(path, cancellable)
          .then(resolve)
          .catch((error) =>
            reject(cancellable.is_cancelled() ? new Cancelled() : error),
          )
          .finally(() => {
            this.active--
            this.pump()
          })
      },
    }
    // Failed decodes aren't cached, so they can be retried.
    promise.catch(() => {
      if (this.entries.get(path) === entry) this.entries.delete(path)
    })
    return entry
  }

  private pump() {
    while (this.active < MAX_DECODES && this.queue.length > 0) {
      this.entries.get(this.queue.shift()!)?.start()
    }
  }

  // Keeps exactly `paths`, decoding in that order (the first, the image shown,
  // goes ahead of any queued preloads).
  keep(paths: string[]) {
    const wanted = new Set(paths)
    let dropped = false
    for (const [path, entry] of this.entries) {
      if (wanted.has(path)) continue
      this.entries.delete(path)
      entry.drop()
      dropped = true
    }
    for (const path of paths) {
      if (!this.entries.has(path)) this.entries.set(path, this.create(path))
    }
    this.queue = paths.filter((path) => !this.entries.get(path)!.started)
    for (const entry of this.entries.values()) entry.promise.catch(() => {})
    this.pump()
    if (dropped) requestGc()
  }

  // The image for `path`, which must be in the last `keep`. Rejects with
  // `Cancelled` if a later `keep` drops it.
  get(path: string) {
    const entry = this.entries.get(path)
    if (!entry) return Promise.reject(new Cancelled())
    return entry.promise
  }

  // For diagnostics and headless tests.
  get size() {
    return this.entries.size
  }

  get decoding() {
    return this.active
  }
}
