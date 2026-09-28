// Types only: the gi:// modules and GJS globals come from these @girs packages
// (package.json), and the bundle's build-time constants are declared here.
import "@girs/gjs"
import "@girs/gjs/dom"
import "@girs/gtk-4.0"
import "@girs/gdkpixbuf-2.0"
import "@girs/gexiv2-0.16"
import "@girs/gly-2"
import "@girs/glygtk4-2"
import "@girs/adw-1"

declare global {
  const ICONS_DIR: string
}
