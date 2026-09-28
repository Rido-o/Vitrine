import GLib from "gi://GLib"
import Gtk from "gi://Gtk?version=4.0"

type Options = {
  seconds: number
  // Whether the controls hide at all (the full-screen view, fullscreen).
  active: () => boolean
  // Keeps them shown, e.g. while a menu is open.
  busy: () => boolean
  setCursorHidden: (hidden: boolean) => void
}

// Fades the controls out and hides the cursor after `seconds` without the
// mouse moving over the watched area, and brings them back on movement.
// Never while the pointer is on a control.
export default class AutoHide {
  private controls: Array<{
    widget: Gtk.Widget
    motion: Gtk.EventControllerMotion
  }> = []
  private timeout = 0
  private lastX = -1
  private lastY = -1

  constructor(private options: Options) {}

  // A control to hide; its "hidden" CSS class fades it.
  add(widget: Gtk.Widget) {
    const motion = new Gtk.EventControllerMotion()
    widget.add_controller(motion)
    this.controls.push({ widget, motion })
  }

  // The area whose mouse movement shows the controls again. GTK also reports
  // motion when widgets change under a still pointer (as when the controls
  // stop taking input), so only a real move counts.
  watch(area: Gtk.Widget) {
    const motion = new Gtk.EventControllerMotion()
    motion.connect("motion", (_c, x, y) => {
      if (x === this.lastX && y === this.lastY) return
      this.lastX = x
      this.lastY = y
      this.show()
    })
    area.add_controller(motion)
  }

  // Shows the controls and cursor, and (when active) hides them again later.
  show() {
    for (const { widget } of this.controls) {
      widget.remove_css_class("hidden")
      widget.canTarget = true
    }
    this.options.setCursorHidden(false)
    this.dispose()
    if (!this.options.active()) return
    this.timeout = GLib.timeout_add_seconds(
      GLib.PRIORITY_DEFAULT,
      this.options.seconds,
      () => {
        this.timeout = 0
        this.hide()
        return GLib.SOURCE_REMOVE
      },
    )
  }

  private hide() {
    if (!this.options.active()) return
    const busy =
      this.options.busy() ||
      this.controls.some(({ motion }) => motion.containsPointer)
    if (busy) return this.show()
    for (const { widget } of this.controls) {
      widget.add_css_class("hidden")
      widget.canTarget = false
    }
    this.options.setCursorHidden(true)
  }

  // Stops a pending hide.
  dispose() {
    if (this.timeout) GLib.source_remove(this.timeout)
    this.timeout = 0
  }
}
