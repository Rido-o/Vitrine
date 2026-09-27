import "./jsx"
import Gdk from "gi://Gdk?version=4.0"
import Gio from "gi://Gio"
import GLib from "gi://GLib"
import Gtk from "gi://Gtk?version=4.0"
import { createRoot } from "gnim"
import { programArgs, programInvocationName } from "system"
import css from "./style.css"
import { pruneThumbnailCache } from "./Thumbnails"
import { APP_ID, APP_NAME, isDirectory } from "./util"
import ViewerWindow from "./Window"

const PRUNE_DELAY_SECONDS = 5

GLib.set_prgname(APP_NAME)
GLib.set_application_name(APP_NAME)

const app = new Gtk.Application({
  applicationId: APP_ID,
  flags: Gio.ApplicationFlags.NON_UNIQUE | Gio.ApplicationFlags.HANDLES_OPEN,
})

let subfolders = false

app.add_main_option(
  "subfolders",
  "r".charCodeAt(0),
  GLib.OptionFlags.NONE,
  GLib.OptionArg.NONE,
  "Include images in subfolders",
  null,
)

app.connect("handle-local-options", (_app, options: GLib.VariantDict) => {
  subfolders = options.contains("subfolders")
  return -1
})

app.connect("startup", () => {
  const display = Gdk.Display.get_default()!
  const provider = new Gtk.CssProvider()
  provider.load_from_string(css)
  Gtk.StyleContext.add_provider_for_display(
    display,
    provider,
    Gtk.STYLE_PROVIDER_PRIORITY_USER,
  )
  Gtk.IconTheme.get_for_display(display).add_search_path(ICONS_DIR)

  // After the first thumbnails, at low priority.
  GLib.timeout_add_seconds(GLib.PRIORITY_LOW, PRUNE_DELAY_SECONDS, () => {
    pruneThumbnailCache().then((pruned) => {
      if (pruned > 0) console.log(`Pruned ${pruned} unused thumbnails`)
    })
    return GLib.SOURCE_REMOVE
  })
})

// Each window gets its own gnim scope, disposed when the window goes away.
function openWindow(directory: string, file: string | null = null) {
  createRoot((dispose) => {
    const win = ViewerWindow(app, directory, file, subfolders)
    win.connect("destroy", dispose)
    win.present()
  })
}

// No argument: ~/Pictures, or the current directory without one.
app.connect("activate", () => {
  const pictures = GLib.get_user_special_dir(
    GLib.UserDirectory.DIRECTORY_PICTURES,
  )
  openWindow(
    pictures && isDirectory(pictures) ? pictures : GLib.get_current_dir(),
  )
})

// A folder opens its grid; a file opens its folder with the file in the
// full-screen view. Only the first argument is used.
app.connect("open", (_app, files: Gio.File[]) => {
  const path = files[0]?.get_path()
  if (!path) return app.activate()
  if (isDirectory(path)) openWindow(path)
  else openWindow(GLib.path_get_dirname(path), path)
})

// runAsync, not run: a blocking run() at module top level stops GJS from
// resolving promises (thumbnails, wallpaper command) while the loop runs.
await app.runAsync([programInvocationName, ...programArgs])
