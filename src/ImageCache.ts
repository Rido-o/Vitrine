import Gdk from "gi://Gdk?version=4.0"
import Gio from "gi://Gio"
import GLib from "gi://GLib"
import System from "system"
import { decodeImage } from "./decode"

// At most this many full-size decodes run at once; the rest wait in a queue,
// so the image being shown never waits behind preloads.
const MAX_DECODES = 2

// Rejection of an entry dropped before its image was needed.
export class Cancelled extends Error {}

type Entry = {
  promise: Promise<Gdk.Texture>
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
export default class ImageCache {
  private entries = new Map<string, Entry>()
  private queue: string[] = []
  private active = 0

  private create(path: string): Entry {
    const cancellable = new Gio.Cancellable()
    let resolve!: (texture: Gdk.Texture) => void
    let reject!: (error: unknown) => void
    const promise = new Promise<Gdk.Texture>((res, rej) => {
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
        decodeImage(path, undefined, cancellable)
          .then((pixbuf) => resolve(Gdk.Texture.new_for_pixbuf(pixbuf)))
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
    if (dropped) {
      GLib.idle_add(GLib.PRIORITY_LOW, () => {
        System.gc()
        return GLib.SOURCE_REMOVE
      })
    }
  }

  // The texture for `path`, which must be in the last `keep`. Rejects with
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
