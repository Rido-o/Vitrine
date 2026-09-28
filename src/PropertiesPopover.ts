import Gtk from "gi://Gtk?version=4.0"
import Pango from "gi://Pango"
import { imageProperties } from "./Properties"

// The i button's popover: the properties of `getPath()`'s image, read each
// time it opens.
export function propertiesPopover(getPath: () => string | null) {
  const content = new Gtk.Box({
    orientation: Gtk.Orientation.VERTICAL,
    spacing: 8,
  })
  const popover = new Gtk.Popover({
    child: content,
    cssClasses: ["viewer-properties"],
  })
  popover.connect("show", () => render(content, getPath()))
  return popover
}

function render(content: Gtk.Box, path: string | null) {
  let child: Gtk.Widget | null
  while ((child = content.get_first_child())) content.remove(child)
  if (!path) {
    content.append(new Gtk.Label({ label: "No image selected" }))
    return
  }
  for (const section of imageProperties(path)) {
    content.append(
      new Gtk.Label({
        label: section.title,
        xalign: 0,
        cssClasses: ["viewer-properties-heading"],
      }),
    )
    const grid = new Gtk.Grid({ columnSpacing: 16, rowSpacing: 4 })
    section.rows.forEach(([key, value], row) => {
      const keyLabel = new Gtk.Label({
        label: key,
        xalign: 1,
        yalign: 0,
        cssClasses: ["viewer-properties-key"],
      })
      // Selectable for copying, but not focusable: the popover would focus
      // the first value and select all of it on opening.
      const valueLabel = new Gtk.Label({
        label: value,
        xalign: 0,
        selectable: true,
        focusable: false,
        wrap: true,
        wrapMode: Pango.WrapMode.WORD_CHAR,
        maxWidthChars: 48,
      })
      grid.attach(keyLabel, 0, row, 1, 1)
      grid.attach(valueLabel, 1, row, 1, 1)
    })
    content.append(grid)
  }
}
