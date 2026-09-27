import Gio from "gi://Gio"
import GLib from "gi://GLib"

export const APP_NAME = "vitrine"
export const APP_TITLE = "Vitrine"
export const APP_ID = "io.github.Rido_o.Vitrine"
// Name before the rename; its cache and history are moved over once.
export const OLD_APP_NAME = "shard-view"

export function cached<T>(
  map: Map<string, T>,
  key: string,
  compute: () => T,
): T {
  const hit = map.get(key)
  if (hit !== undefined) return hit
  const value = compute()
  map.set(key, value)
  return value
}

export function normalizeDirectory(input: string) {
  const path = input.trim()
  const expanded =
    path === "~" || path.startsWith("~/")
      ? GLib.get_home_dir() + path.slice(1)
      : path
  return GLib.canonicalize_filename(expanded, GLib.get_current_dir())
}

export function isDirectory(path: string) {
  return GLib.file_test(path, GLib.FileTest.IS_DIR)
}

export function basename(path: string) {
  return GLib.path_get_basename(path)
}

export function showInFileManager(file: string) {
  const uri = Gio.File.new_for_path(file).get_uri()
  Gio.DBus.session.call(
    "org.freedesktop.FileManager1",
    "/org/freedesktop/FileManager1",
    "org.freedesktop.FileManager1",
    "ShowItems",
    new GLib.Variant("(ass)", [[uri], ""]),
    null,
    Gio.DBusCallFlags.NONE,
    -1,
    null,
    (connection, result) => {
      try {
        connection!.call_finish(result)
      } catch (error) {
        console.error(`Could not show ${file} in file manager:`, error)
      }
    },
  )
}

// VITRINE_WALLPAPER_COMMAND, e.g. "set-wallpaper"; the image path is
// appended as the last argument. Unset hides the "Set wallpaper" action.
export function wallpaperCommand(): string[] | null {
  const command = GLib.getenv("VITRINE_WALLPAPER_COMMAND")?.trim()
  if (!command) return null
  try {
    const [, argv] = GLib.shell_parse_argv(command)
    return argv.length > 0 ? argv : null
  } catch (error) {
    console.error("Invalid VITRINE_WALLPAPER_COMMAND:", error)
    return null
  }
}

export function setWallpaper(argv: string[], file: string): Promise<void> {
  return new Promise((resolve, reject) => {
    try {
      const process = Gio.Subprocess.new(
        [...argv, file],
        Gio.SubprocessFlags.NONE,
      )
      process.wait_check_async(null, (_process, result) => {
        try {
          process.wait_check_finish(result)
          resolve()
        } catch (error) {
          reject(error)
        }
      })
    } catch (error) {
      reject(error)
    }
  })
}
