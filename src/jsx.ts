import Gtk from "gi://Gtk?version=4.0"
import type { CCProps } from "gnim"
import { intrinsicElements } from "gnim/gtk4/jsx-runtime"

Object.assign(intrinsicElements, {
  box: Gtk.Box,
  button: Gtk.Button,
  entry: Gtk.Entry,
  image: Gtk.Image,
  label: Gtk.Label,
  overlay: Gtk.Overlay,
  scrolledwindow: Gtk.ScrolledWindow,
  stack: Gtk.Stack,
  togglebutton: Gtk.ToggleButton,
})

type Props<T extends Gtk.Widget, P> = CCProps<T, Partial<P>>

declare global {
  namespace JSX {
    interface IntrinsicElements {
      box: Props<Gtk.Box, Gtk.Box.ConstructorProps>
      button: Props<Gtk.Button, Gtk.Button.ConstructorProps>
      entry: Props<Gtk.Entry, Gtk.Entry.ConstructorProps>
      image: Props<Gtk.Image, Gtk.Image.ConstructorProps>
      label: Props<Gtk.Label, Gtk.Label.ConstructorProps>
      overlay: Props<Gtk.Overlay, Gtk.Overlay.ConstructorProps>
      scrolledwindow: Props<
        Gtk.ScrolledWindow,
        Gtk.ScrolledWindow.ConstructorProps
      >
      stack: Props<Gtk.Stack, Gtk.Stack.ConstructorProps>
      togglebutton: Props<Gtk.ToggleButton, Gtk.ToggleButton.ConstructorProps>
    }
  }
}
