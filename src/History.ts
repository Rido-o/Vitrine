import GLib from "gi://GLib"
import { APP_NAME, isDirectory } from "./util"

const LIMIT = 10
const FILE = GLib.build_filenamev([
  GLib.get_user_state_dir(),
  APP_NAME,
  "history",
])
// Read once if the new file doesn't exist yet.
const OLD_FILE = GLib.build_filenamev([
  GLib.get_user_state_dir(),
  "ags",
  "wallpaper-history",
])

function read(path: string) {
  try {
    const [, contents] = GLib.file_get_contents(path)
    return new TextDecoder().decode(contents).split("\n")
  } catch {
    return null
  }
}

// Recently opened folders, most recent first.
export default class History {
  readonly entries: string[]

  constructor() {
    const lines = read(FILE) ?? read(OLD_FILE) ?? []
    this.entries = [
      ...new Set(lines.filter((line) => line && isDirectory(line))),
    ].slice(0, LIMIT)
  }

  remember(directory: string) {
    const index = this.entries.indexOf(directory)
    if (index !== -1) this.entries.splice(index, 1)
    this.entries.unshift(directory)
    this.entries.splice(LIMIT)
    try {
      GLib.mkdir_with_parents(GLib.path_get_dirname(FILE), 0o755)
      GLib.file_set_contents(FILE, this.entries.join("\n") + "\n")
    } catch (error) {
      console.error("Could not save folder history:", error)
    }
  }
}
