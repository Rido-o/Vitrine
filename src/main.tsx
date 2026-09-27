import "./jsx"
import Gdk from "gi://Gdk?version=4.0"
import Gio from "gi://Gio"
import Gtk from "gi://Gtk?version=4.0"
import { programArgs, programInvocationName } from "system"
import css from "./style.css"

declare const ICONS_DIR: string

const app = new Gtk.Application({
  applicationId: "dev.shard.View",
  flags: Gio.ApplicationFlags.NON_UNIQUE,
})

app.connect("activate", () => {
  const display = Gdk.Display.get_default()!
  const provider = new Gtk.CssProvider()
  provider.load_from_string(css)
  Gtk.StyleContext.add_provider_for_display(
    display,
    provider,
    Gtk.STYLE_PROVIDER_PRIORITY_USER,
  )
  Gtk.IconTheme.get_for_display(display).add_search_path(ICONS_DIR)

  const win = (
    <Gtk.ApplicationWindow
      application={app}
      title="shard-view"
      defaultWidth={1200}
      defaultHeight={800}
    >
      <box
        class="shard-view"
        halign={Gtk.Align.CENTER}
        valign={Gtk.Align.CENTER}
        spacing={8}
      >
        <image iconName="image-awesome-symbolic" pixelSize={24} />
        <label label="shard-view" />
      </box>
    </Gtk.ApplicationWindow>
  ) as Gtk.ApplicationWindow
  win.present()
})

app.run([programInvocationName, ...programArgs])
