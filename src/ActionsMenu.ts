import Gio from "gi://Gio"
import GLib from "gi://GLib"
import Gtk from "gi://Gtk?version=4.0"

export type MenuAction = {
  name: string
  label: string
  // Only labels the item: the keys are handled by the window's key handler
  // (so Ctrl+C still copies text in the folder entry).
  accel?: string
  activate: () => void
  // Only enabled in the full-screen view.
  viewOnly?: boolean
}

// The ⋯ menu: a menu model of window actions ("win.…"), one section per
// group. The model exists before the window, so the actions are added to it
// with `addTo` once it does.
export default class ActionsMenu {
  readonly model = new Gio.Menu()
  private actions: Gio.SimpleAction[] = []
  private viewActions: Gio.SimpleAction[] = []

  constructor(sections: MenuAction[][]) {
    for (const section of sections) {
      const menu = new Gio.Menu()
      for (const { name, label, accel, activate, viewOnly } of section) {
        const item = Gio.MenuItem.new(label, `win.${name}`)
        if (accel) {
          item.set_attribute_value("accel", new GLib.Variant("s", accel))
        }
        menu.append_item(item)
        const action = new Gio.SimpleAction({ name, enabled: !viewOnly })
        action.connect("activate", activate)
        this.actions.push(action)
        if (viewOnly) this.viewActions.push(action)
      }
      this.model.append_section(null, menu)
    }
  }

  addTo(window: Gtk.ApplicationWindow) {
    for (const action of this.actions) window.add_action(action)
  }

  setInView(inView: boolean) {
    for (const action of this.viewActions) action.enabled = inView
  }
}
