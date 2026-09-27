import Gtk from "gi://Gtk?version=4.0"
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
