import Gio from "gi://Gio"
import GLib from "gi://GLib"
import Gtk from "gi://Gtk?version=4.0"
import { evictThumbnail } from "./Thumbnails"
import { cached } from "./util"

export type SortKey = "name" | "date" | "size" | "random"

const IMAGE_EXTENSIONS = new Set([
  "gif",
  "jpeg",
  "jpg",
  "png",
  "tif",
  "tiff",
  "webp",
])
const ATTRIBUTES =
  "standard::name,standard::type,standard::is-symlink,standard::size,time::modified"
const BATCH_SIZE = 200
const SCAN_WORKERS = 4
// Folders watched for changes (inotify watches are limited), and how often
// changes trigger a rescan at most.
const WATCH_LIMIT = 1000
const RESCAN_DELAY_MS = 1000
const WATCHED_EVENTS = new Set([
  Gio.FileMonitorEvent.CHANGES_DONE_HINT,
  Gio.FileMonitorEvent.CREATED,
  Gio.FileMonitorEvent.DELETED,
  Gio.FileMonitorEvent.MOVED_IN,
  Gio.FileMonitorEvent.MOVED_OUT,
  Gio.FileMonitorEvent.RENAMED,
])

export function isImage(path: string) {
  return IMAGE_EXTENSIONS.has(path.split(".").pop()?.toLowerCase() ?? "")
}

function enumerateChildren(directory: Gio.File, cancellable: Gio.Cancellable) {
  return new Promise<Gio.FileEnumerator>((resolve, reject) => {
    directory.enumerate_children_async(
      ATTRIBUTES,
      Gio.FileQueryInfoFlags.NONE,
      GLib.PRIORITY_DEFAULT,
      cancellable,
      (_source, result) => {
        try {
          resolve(directory.enumerate_children_finish(result))
        } catch (error) {
          reject(error)
        }
      },
    )
  })
}

function nextFiles(
  enumerator: Gio.FileEnumerator,
  cancellable: Gio.Cancellable,
) {
  return new Promise<Gio.FileInfo[]>((resolve, reject) => {
    enumerator.next_files_async(
      BATCH_SIZE,
      GLib.PRIORITY_DEFAULT,
      cancellable,
      (_source, result) => {
        try {
          resolve(enumerator.next_files_finish(result))
        } catch (error) {
          reject(error)
        }
      },
    )
  })
}

// An ordered list of the images in a folder, mirrored into a Gtk.StringList
// for the grid. Scanning is asynchronous: images are inserted in sort order
// as batches arrive, and `onChanged` runs after each batch.
export default class Library {
  readonly paths: string[] = []
  readonly model = new Gtk.StringList()
  directory = ""
  recursive = false
  sortKey: SortKey = "name"
  descending = false
  loading = false
  onChanged: () => void = () => {}
  // After a rescan that completed, whether asked for or after a change on disk.
  onRescanned: () => void = () => {}
  // For each image a rescan found changed on disk.
  onModified: (path: string) => void = () => {}

  private mtimes = new Map<string, number>()
  private sizes = new Map<string, number>()
  private randomKeys = new Map<string, number>()
  private cancellable: Gio.Cancellable | null = null
  // Images removed (trashed) or added (restored) in Vitrine while a scan runs;
  // a rescan's results predate them, so it leaves them alone.
  private removedDuringScan = new Set<string>()
  private addedDuringScan = new Set<string>()
  private monitors = new Map<string, Gio.FileMonitor>()
  private rescanTimeout = 0

  mtime(path: string) {
    return this.mtimes.get(path) ?? 0
  }

  compare = (a: string, b: string) => {
    let result = 0
    if (this.sortKey === "date") {
      result = this.mtime(a) - this.mtime(b)
    } else if (this.sortKey === "size") {
      result = (this.sizes.get(a) ?? 0) - (this.sizes.get(b) ?? 0)
    } else if (this.sortKey === "random") {
      result =
        cached(this.randomKeys, a, Math.random) -
        cached(this.randomKeys, b, Math.random)
    }
    if (result === 0) result = a < b ? -1 : a > b ? 1 : 0
    return this.descending && this.sortKey !== "random" ? -result : result
  }

  private insertionIndex(path: string) {
    let low = 0
    let high = this.paths.length
    while (low < high) {
      const middle = (low + high) >> 1
      if (this.compare(this.paths[middle], path) < 0) low = middle + 1
      else high = middle
    }
    return low
  }

  private insert(path: string) {
    const at = this.insertionIndex(path)
    this.paths.splice(at, 0, path)
    this.model.splice(at, 0, [path])
  }

  // Walks `root` (and its subfolders when recursive, SCAN_WORKERS folders at a
  // time: over NFS each call is a round trip), calling `onBatch` with the
  // image paths found in each batch; returns the folders read.
  private async scan(
    root: Gio.File,
    cancellable: Gio.Cancellable,
    onBatch: (images: string[]) => void,
  ) {
    const directories: Gio.File[] = []
    const pending = [root]
    let active = 0
    await new Promise<void>((resolve) => {
      const pump = () => {
        if (
          cancellable.is_cancelled() ||
          (pending.length === 0 && active === 0)
        )
          return resolve()
        while (active < SCAN_WORKERS && pending.length > 0) {
          const directory = pending.shift()!
          active++
          this.scanDirectory(directory, pending, cancellable, onBatch)
            .then((read) => read && directories.push(directory))
            .finally(() => {
              active--
              pump()
            })
        }
      }
      pump()
    })
    return directories
  }

  private async scanDirectory(
    directory: Gio.File,
    pending: Gio.File[],
    cancellable: Gio.Cancellable,
    onBatch: (images: string[]) => void,
  ) {
    let enumerator: Gio.FileEnumerator
    try {
      enumerator = await enumerateChildren(directory, cancellable)
    } catch (error) {
      if (cancellable.is_cancelled()) return
      console.error(`Could not read ${directory.get_path()}:`, error)
      return
    }
    let read = true
    try {
      let infos: Gio.FileInfo[]
      while ((infos = await nextFiles(enumerator, cancellable)).length > 0) {
        const images: string[] = []
        for (const info of infos) {
          const child = directory.get_child(info.get_name())
          if (info.get_file_type() === Gio.FileType.DIRECTORY) {
            if (this.recursive && !info.get_is_symlink()) pending.push(child)
            continue
          }
          const path = child.get_path()
          if (!path || !isImage(path)) continue
          this.mtimes.set(
            path,
            info.get_modification_date_time()?.to_unix() ?? 0,
          )
          this.sizes.set(path, info.get_size())
          images.push(path)
        }
        if (cancellable.is_cancelled()) return
        if (images.length > 0) onBatch(images)
      }
    } catch (error) {
      if (cancellable.is_cancelled()) return
      console.error(`Could not read ${directory.get_path()}:`, error)
      read = false
    } finally {
      enumerator.close_async(GLib.PRIORITY_DEFAULT, null, null)
    }
    return read
  }

  private begin() {
    this.cancellable?.cancel()
    const cancellable = new Gio.Cancellable()
    this.cancellable = cancellable
    this.loading = true
    this.removedDuringScan.clear()
    this.addedDuringScan.clear()
    return cancellable
  }

  private finish(cancellable: Gio.Cancellable) {
    if (cancellable !== this.cancellable) return false
    this.loading = false
    this.cancellable = null
    this.onChanged()
    return true
  }

  // Replaces the list with `directory`'s images; resolves once the scan is
  // complete (or false if another load or rescan superseded it).
  async load(directory: string) {
    const cancellable = this.begin()
    this.unwatch()
    this.directory = directory
    this.paths.length = 0
    this.model.splice(0, this.model.get_n_items(), [])
    this.onChanged()

    const directories = await this.scan(
      Gio.File.new_for_path(directory),
      cancellable,
      (images) => {
        for (const path of images) {
          if (this.removedDuringScan.has(path)) this.evict(path)
          else if (!this.addedDuringScan.has(path)) this.insert(path)
        }
        this.onChanged()
      },
    )
    if (!this.finish(cancellable)) return false
    this.watch(directories)
    return true
  }

  // Picks up added, removed and changed files without rebuilding the model.
  async rescan() {
    const cancellable = this.begin()
    const previousMtimes = new Map(this.mtimes)
    const fresh: string[] = []
    const directories = await this.scan(
      Gio.File.new_for_path(this.directory),
      cancellable,
      (images) => fresh.push(...images),
    )
    if (cancellable.is_cancelled()) return false

    const freshSet = new Set(fresh)
    for (let i = this.paths.length - 1; i >= 0; i--) {
      const path = this.paths[i]
      if (!freshSet.has(path) && !this.addedDuringScan.has(path)) {
        this.paths.splice(i, 1)
        this.model.remove(i)
        this.evict(path)
      }
    }

    const current = new Set(this.paths)
    for (const path of fresh) {
      if (this.removedDuringScan.has(path)) this.evict(path)
      else if (!current.has(path)) this.insert(path)
    }

    let modified = false
    for (const path of fresh) {
      const previous = previousMtimes.get(path)
      if (previous === undefined || previous === this.mtimes.get(path)) continue
      modified = true
      evictThumbnail(path)
      this.onModified(path)
      const index = this.paths.indexOf(path)
      if (index !== -1) this.model.splice(index, 1, [path])
    }
    if (modified && (this.sortKey === "date" || this.sortKey === "size")) {
      this.sort()
    }
    if (!this.finish(cancellable)) return false
    this.watch(directories)
    this.onRescanned()
    return true
  }

  sort() {
    this.paths.sort(this.compare)
    this.model.splice(0, this.model.get_n_items(), this.paths)
  }

  setSort(key: SortKey) {
    if (key === "random") this.randomKeys.clear()
    else if (key !== this.sortKey)
      this.descending = key === "date" || key === "size"
    this.sortKey = key
    this.sort()
  }

  toggleDirection() {
    this.descending = !this.descending
    this.sort()
  }

  // Whether `path` belongs in this list (the folder, or below it when
  // recursive).
  contains(path: string) {
    const parent = GLib.path_get_dirname(path)
    return (
      isImage(path) &&
      (parent === this.directory ||
        (this.recursive && path.startsWith(this.directory + "/")))
    )
  }

  // Inserts a file that appeared outside a scan (e.g. restored from the
  // trash); returns its index, or -1 if it doesn't belong here.
  add(path: string) {
    if (!this.contains(path) || this.paths.includes(path)) {
      return this.paths.indexOf(path)
    }
    try {
      const info = Gio.File.new_for_path(path).query_info(
        ATTRIBUTES,
        Gio.FileQueryInfoFlags.NONE,
        null,
      )
      this.mtimes.set(path, info.get_modification_date_time()?.to_unix() ?? 0)
      this.sizes.set(path, info.get_size())
    } catch (error) {
      console.error(`Could not read ${path}:`, error)
      return -1
    }
    this.insert(path)
    if (this.loading) {
      this.addedDuringScan.add(path)
      this.removedDuringScan.delete(path)
    }
    return this.paths.indexOf(path)
  }

  remove(path: string) {
    const index = this.paths.indexOf(path)
    if (index !== -1) {
      this.paths.splice(index, 1)
      this.model.remove(index)
    }
    this.evict(path)
    if (this.loading) {
      this.removedDuringScan.add(path)
      this.addedDuringScan.delete(path)
    }
    return index
  }

  // Watches the scanned folders (the first WATCH_LIMIT) and rescans after a
  // change. Monitors only see changes made on this machine (not, say, on an
  // NFS server).
  private watch(directories: Gio.File[]) {
    const wanted = new Map(
      directories.slice(0, WATCH_LIMIT).map((dir) => [dir.get_path()!, dir]),
    )
    for (const [path, monitor] of this.monitors) {
      if (wanted.has(path)) continue
      monitor.cancel()
      this.monitors.delete(path)
    }
    for (const [path, directory] of wanted) {
      if (this.monitors.has(path)) continue
      try {
        const monitor = directory.monitor_directory(
          Gio.FileMonitorFlags.WATCH_MOVES,
          null,
        )
        monitor.connect("changed", (_monitor, file, other, event) =>
          this.onFileChanged(file, other, event),
        )
        this.monitors.set(path, monitor)
      } catch (error) {
        console.error(`Could not watch ${path}:`, error)
      }
    }
  }

  private unwatch() {
    for (const monitor of this.monitors.values()) monitor.cancel()
    this.monitors.clear()
    if (this.rescanTimeout) GLib.source_remove(this.rescanTimeout)
    this.rescanTimeout = 0
  }

  // Other files only matter when recursive: they may be folders.
  private onFileChanged(
    file: Gio.File,
    other: Gio.File | null,
    event: Gio.FileMonitorEvent,
  ) {
    if (!WATCHED_EVENTS.has(event)) return
    const relevant =
      this.recursive ||
      isImage(file.get_basename() ?? "") ||
      isImage(other?.get_basename() ?? "")
    if (relevant) this.scheduleRescan()
  }

  // At most one rescan per RESCAN_DELAY_MS, so copying many files doesn't
  // rescan for each; waits for a running scan to finish first.
  private scheduleRescan() {
    if (this.rescanTimeout) return
    this.rescanTimeout = GLib.timeout_add(
      GLib.PRIORITY_DEFAULT,
      RESCAN_DELAY_MS,
      () => {
        this.rescanTimeout = 0
        if (this.loading) this.scheduleRescan()
        else this.rescan()
        return GLib.SOURCE_REMOVE
      },
    )
  }

  // Stops scanning and watching, for a closed window.
  dispose() {
    this.cancellable?.cancel()
    this.unwatch()
  }

  private evict(path: string) {
    this.mtimes.delete(path)
    this.sizes.delete(path)
    evictThumbnail(path)
  }
}
