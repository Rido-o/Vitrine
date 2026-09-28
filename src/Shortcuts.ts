import Gdk from "gi://Gdk?version=4.0"
import Gtk from "gi://Gtk?version=4.0"

type Section = { title: string; rows: Array<[string, string]> }

// Keep in step with onKey in Window.tsx and the README's key table.
function sections(wallpaper: boolean): Section[] {
  const all: Section[] = [
    {
      title: "Grid and full-screen view",
      rows: [
        ["i", "Image properties"],
        ["Ctrl+C", "Copy image"],
        ["Ctrl+Shift+C", "Copy path"],
        ["w", "Set as wallpaper"],
        ["r", "Rescan folder"],
        ["Delete", "Move to trash"],
        ["Ctrl+Z", "Undo delete"],
        ["?", "Keyboard shortcuts"],
      ],
    },
    {
      title: "Grid",
      rows: [
        ["Enter, double-click", "Open in the full-screen view"],
        ["Esc, q", "Quit"],
      ],
    },
    {
      title: "Full-screen view",
      rows: [
        ["← →, click the edges", "Previous/next image"],
        ["Scroll, drag", "Zoom around the cursor, pan"],
        ["Double-click", "Toggle fit/100%"],
        ["+ − 0", "Zoom in, out, fit"],
        ["s", "Sharp pixels"],
        ["f", "Toggle fullscreen"],
        ["Esc, q", "Back to the grid"],
      ],
    },
  ]
  // "w" does nothing without a wallpaper command.
  return all.map(({ title, rows }) => ({
    title,
    rows: rows.filter(([keys]) => wallpaper || keys !== "w"),
  }))
}

// A modal window listing the keys; Esc, q or ? closes it.
export function showShortcuts(parent: Gtk.Window, wallpaper: boolean) {
  const content = new Gtk.Box({
    orientation: Gtk.Orientation.VERTICAL,
    spacing: 8,
    cssClasses: ["viewer-shortcuts-content"],
  })
  for (const section of sections(wallpaper)) {
    content.append(
      new Gtk.Label({
        label: section.title,
        xalign: 0,
        cssClasses: ["viewer-shortcuts-heading"],
      }),
    )
    const grid = new Gtk.Grid({ columnSpacing: 16, rowSpacing: 6 })
    section.rows.forEach(([keys, description], row) => {
      grid.attach(
        new Gtk.Label({ label: keys, xalign: 0, cssClasses: ["key"] }),
        0,
        row,
        1,
        1,
      )
      grid.attach(
        new Gtk.Label({ label: description, xalign: 0, hexpand: true }),
        1,
        row,
        1,
        1,
      )
    })
    content.append(grid)
  }

  const window = new Gtk.Window({
    title: "Keyboard shortcuts",
    transientFor: parent,
    modal: true,
    defaultWidth: 520,
    defaultHeight: 620,
    cssClasses: ["viewer-shortcuts"],
    child: new Gtk.ScrolledWindow({
      hscrollbarPolicy: Gtk.PolicyType.NEVER,
      child: content,
    }),
  })
  const keys = new Gtk.EventControllerKey()
  keys.connect("key-pressed", (_c, keyval) => {
    if (
      keyval !== Gdk.KEY_Escape &&
      keyval !== Gdk.KEY_q &&
      keyval !== Gdk.KEY_question
    )
      return false
    window.close()
    return true
  })
  window.add_controller(keys)
  window.present()
}
