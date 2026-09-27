import Gio from "gi://Gio"
import Gtk from "gi://Gtk?version=4.0"
import { evictThumbnail } from "./Thumbnails"
import { cached } from "./util"

export type SortKey = "name" | "date" | "size" | "random"

const IMAGE_EXTENSIONS = new Set(["png", "jpg", "jpeg", "webp"])

export function isImage(path: string) {
  return IMAGE_EXTENSIONS.has(path.split(".").pop()?.toLowerCase() ?? "")
}

// An ordered list of the images in a folder, mirrored into a Gtk.StringList
// for the grid.
export default class Library {
  readonly paths: string[] = []
  readonly model = new Gtk.StringList()
  directory = ""
  recursive = false
  sortKey: SortKey = "name"
  descending = false

  private mtimes = new Map<string, number>()
  private sizes = new Map<string, number>()
  private randomKeys = new Map<string, number>()

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

  private scan(directory: Gio.File, images: string[] = []) {
    try {
      const enumerator = directory.enumerate_children(
        "standard::name,standard::type,standard::is-symlink,standard::size,time::modified",
        Gio.FileQueryInfoFlags.NONE,
        null,
      )
      let info: Gio.FileInfo | null
      while ((info = enumerator.next_file(null))) {
        const child = directory.get_child(info.get_name())
        if (info.get_file_type() === Gio.FileType.DIRECTORY) {
          if (this.recursive && !info.get_is_symlink()) this.scan(child, images)
          continue
        }
        const path = child.get_path()
        if (path && isImage(path)) {
          this.mtimes.set(
            path,
            info.get_modification_date_time()?.to_unix() ?? 0,
          )
          this.sizes.set(path, info.get_size())
          images.push(path)
        }
      }
      enumerator.close(null)
    } catch (error) {
      console.error(`Could not read ${directory.get_path()}:`, error)
    }
    return images
  }

  load(directory: string) {
    this.directory = directory
    const fresh = this.scan(Gio.File.new_for_path(directory)).sort(this.compare)
    this.paths.length = 0
    this.paths.push(...fresh)
    this.model.splice(0, this.model.get_n_items(), fresh)
  }

  // Picks up added, removed and changed files without rebuilding the model.
  rescan() {
    const previousMtimes = new Map(this.mtimes)
    const fresh = this.scan(Gio.File.new_for_path(this.directory))
    const freshSet = new Set(fresh)
    const previousSet = new Set(this.paths)

    for (let i = this.paths.length - 1; i >= 0; i--) {
      const path = this.paths[i]
      if (!freshSet.has(path)) {
        this.paths.splice(i, 1)
        this.model.remove(i)
        this.evict(path)
      }
    }

    for (const path of fresh) {
      if (previousSet.has(path)) continue
      const next = this.paths.findIndex(
        (existing) => this.compare(existing, path) > 0,
      )
      const at = next === -1 ? this.paths.length : next
      this.paths.splice(at, 0, path)
      this.model.splice(at, 0, [path])
    }

    let modified = false
    for (const path of fresh) {
      const previous = previousMtimes.get(path)
      if (previous === undefined || previous === this.mtimes.get(path)) continue
      modified = true
      evictThumbnail(path)
      const index = this.paths.indexOf(path)
      if (index !== -1) this.model.splice(index, 1, [path])
    }
    if (modified && (this.sortKey === "date" || this.sortKey === "size")) {
      this.sort()
    }
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

  remove(path: string) {
    const index = this.paths.indexOf(path)
    if (index !== -1) {
      this.paths.splice(index, 1)
      this.model.remove(index)
    }
    this.evict(path)
    return index
  }

  private evict(path: string) {
    this.mtimes.delete(path)
    this.sizes.delete(path)
    evictThumbnail(path)
  }
}
