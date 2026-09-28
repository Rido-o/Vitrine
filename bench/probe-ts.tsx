// The TypeScript app's benchmark (see README.md): opens the real window on a
// folder (with subfolders), runs the scenarios and prints RESULT lines in the
// same format as rust/src/probe.rs, then quits. Built by `nix build
// .#bench-ts`.
import "../src/jsx"
import Gdk from "gi://Gdk?version=4.0"
import Gio from "gi://Gio"
import GLib from "gi://GLib"
import GObject from "gi://GObject"
import Gtk from "gi://Gtk?version=4.0"
import { createRoot } from "gnim"
import { programArgs } from "system"
import css from "../src/style.css"
import ViewerWindow from "../src/Window"

const STALL_US = 8_000
// A frame is late when it takes over 1.5 refresh intervals.
const LATE_MS = (1.5 * 1000) / Number(GLib.getenv("VITRINE_PROBE_HZ") ?? 60)
const SCROLL_PX_PER_S = 4000
const SCAN_SETTLE_MS = 1000
const FILL_TIMEOUT_MS = 60_000
const OPEN_POSITION = 0
const HOLD_PRESSES = 60
const HOLD_INTERVAL_MS = 33

function recorder(widget: Gtk.Widget) {
  const frames: number[] = []
  const stalls: number[] = []
  const tick = widget.add_tick_callback((_w, clock) => {
    frames.push(clock.get_frame_time())
    return GLib.SOURCE_CONTINUE
  })
  let last = GLib.get_monotonic_time()
  const timer = GLib.timeout_add(GLib.PRIORITY_HIGH, 2, () => {
    const now = GLib.get_monotonic_time()
    if (now - last > STALL_US) stalls.push(now - last)
    last = now
    return GLib.SOURCE_CONTINUE
  })
  return (label: string, extra = "") => {
    widget.remove_tick_callback(tick)
    GLib.source_remove(timer)
    const intervals = frames
      .slice(1)
      .map((t, i) => (t - frames[i]) / 1000)
      .sort((a, b) => a - b)
    const pct = (p: number) =>
      intervals.length
        ? intervals[Math.round((intervals.length - 1) * p)]
        : 0
    const sum = stalls.reduce((a, b) => a + b, 0)
    print(
      `RESULT ${label} frames=${frames.length} p50=${pct(0.5).toFixed(1)} ` +
        `p95=${pct(0.95).toFixed(1)} p99=${pct(0.99).toFixed(1)} ` +
        `max=${pct(1).toFixed(1)} ` +
        `over25=${intervals.filter((ms) => ms > 25).length} ` +
        `late=${intervals.filter((ms) => ms > LATE_MS).length} ` +
        `stalls=${stalls.length} stall_sum=${Math.floor(sum / 1000)} ` +
        `stall_max=${Math.floor(Math.max(0, ...stalls) / 1000)} ${extra}`,
    )
  }
}

function findAll<T>(root: Gtk.Widget, test: (w: Gtk.Widget) => boolean) {
  const found: T[] = []
  const walk = (w: Gtk.Widget) => {
    if (test(w)) found.push(w as T)
    for (let c = w.get_first_child(); c; c = c.get_next_sibling()) walk(c)
  }
  walk(root)
  return found
}

const find = <T,>(root: Gtk.Widget, test: (w: Gtk.Widget) => boolean) =>
  findAll<T>(root, test)[0] ?? null

const msSince = (start: number) =>
  Math.floor((GLib.get_monotonic_time() - start) / 1000)

const sleep = (ms: number) =>
  new Promise<void>((resolve) =>
    GLib.timeout_add(GLib.PRIORITY_DEFAULT, ms, () => {
      resolve()
      return GLib.SOURCE_REMOVE
    }),
  )

async function waitUntil(timeoutMs: number, done: () => boolean) {
  const start = GLib.get_monotonic_time()
  while (!done()) {
    if (msSince(start) > timeoutMs) return -1
    await sleep(5)
  }
  return msSince(start)
}

function tilesFilled(grid: Gtk.GridView) {
  const pictures = findAll<Gtk.Picture>(grid, (w) => w instanceof Gtk.Picture)
  return pictures.length > 0 && pictures.every((p) => p.paintable !== null)
}

function memory() {
  const [, bytes] = GLib.file_get_contents("/proc/self/status")
  const status = new TextDecoder().decode(bytes)
  const field = (name: string) =>
    Math.floor(Number(status.match(new RegExp(`${name}\\s+(\\d+)`))![1]) / 1024)
  return `rss_mb=${field("VmRSS:")} hwm_mb=${field("VmHWM:")}`
}

// The full-screen view's private state (ZoomableImage.ts).
type Preview = Gtk.Widget & {
  path: string | null
  texture: Gdk.Texture | null
  showingPlaceholder: boolean
}

async function run(win: Gtk.ApplicationWindow, start: number) {
  let done = recorder(win)
  let grid: Gtk.GridView | null = null
  await waitUntil(1000, () => {
    grid = find<Gtk.GridView>(win, (w) => w instanceof Gtk.GridView)
    return grid !== null
  })
  const g = grid! as Gtk.GridView
  const count = () => g.model?.get_n_items() ?? 0
  let first = -1
  let lastCount = 0
  let lastChange = GLib.get_monotonic_time()
  for (;;) {
    const n = count()
    const now = GLib.get_monotonic_time()
    if (n !== lastCount) {
      if (first < 0 && n > 0) first = Math.floor((now - start) / 1000)
      lastCount = n
      lastChange = now
    } else if (n > 0 && (now - lastChange) / 1000 > SCAN_SETTLE_MS) break
    await sleep(5)
  }
  done(
    "scan",
    `first_ms=${first} done_ms=${Math.floor((lastChange - start) / 1000)} ` +
      `items=${lastCount}`,
  )

  const adjustment = (g.get_parent() as Gtk.ScrolledWindow).vadjustment
  print(
    `RESULT fill_first fill_ms=${await waitUntil(FILL_TIMEOUT_MS, () =>
      tilesFilled(g),
    )}`,
  )

  done = recorder(win)
  const scrollStart = GLib.get_monotonic_time()
  const end = adjustment.upper - adjustment.pageSize
  while (adjustment.value < end) {
    const elapsed = (GLib.get_monotonic_time() - scrollStart) / 1e6
    adjustment.value = Math.min(elapsed * SCROLL_PX_PER_S, end)
    await sleep(4)
  }
  let fill = await waitUntil(FILL_TIMEOUT_MS, () => tilesFilled(g))
  done("scroll", `scroll_ms=${msSince(scrollStart)} fill_after_ms=${fill}`)

  done = recorder(win)
  adjustment.value = end / 2
  fill = await waitUntil(FILL_TIMEOUT_MS, () => tilesFilled(g))
  done("jump", `fill_ms=${fill}`)

  // open: from the top, the first image.
  adjustment.value = 0
  // Let the grid rebind its tiles first (they still show the middle).
  await sleep(200)
  await waitUntil(FILL_TIMEOUT_MS, () => tilesFilled(g))
  const preview = find<Preview>(win, (w) =>
    GObject.type_name(w.constructor.$gtype).includes("ZoomableImage"),
  )!
  const pathAt = (i: number) =>
    (g.model!.get_item(i) as Gtk.StringObject).get_string()
  const sharp = (path: string) => () =>
    preview.path === path && !!preview.texture && !preview.showingPlaceholder
  done = recorder(win)
  const openStart = GLib.get_monotonic_time()
  g.emit("activate", OPEN_POSITION)
  const placeholder = await waitUntil(5000, () => !!preview.texture)
  const sharpOpen = await waitUntil(5000, sharp(pathAt(OPEN_POSITION)))
  const sharpMs = sharpOpen < 0 ? -1 : msSince(openStart)
  await sleep(Math.max(0, 1500 - msSince(openStart)))
  done("open", `placeholder_ms=${placeholder} sharp_ms=${sharpMs}`)

  // hold: → at 30 presses/s, then how long the last image takes.
  // GTK adds key controllers of its own; like GTK, stop at the first that
  // handles the key.
  const controllers = win.observe_controllers()
  const keys: Gtk.EventControllerKey[] = []
  for (let i = 0; i < controllers.get_n_items(); i++) {
    const c = controllers.get_item(i)
    if (c instanceof Gtk.EventControllerKey) keys.push(c)
  }
  const press = (keyval: number) =>
    keys.some(
      (c) =>
        c.emit("key-pressed", keyval, 0, 0 as Gdk.ModifierType) as boolean,
    )
  done = recorder(win)
  for (let i = 0; i < HOLD_PRESSES; i++) {
    press(Gdk.KEY_Right)
    await sleep(HOLD_INTERVAL_MS)
  }
  const last = pathAt((OPEN_POSITION + HOLD_PRESSES) % count())
  const settle = await waitUntil(5000, sharp(last))
  done("hold", `presses=${HOLD_PRESSES} last_sharp_ms=${settle}`)

  done = recorder(win)
  press(Gdk.KEY_Escape)
  await sleep(1000)
  done("close")

  done = recorder(win)
  await sleep(2000)
  done("idle")
  print(`RESULT memory ${memory()}`)
}

const app = new Gtk.Application({
  applicationId: "io.github.Rido_o.Vitrine.Bench",
  flags: Gio.ApplicationFlags.NON_UNIQUE,
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
})

app.connect("activate", () => {
  createRoot((dispose) => {
    const start = GLib.get_monotonic_time()
    const win = ViewerWindow(app, programArgs[0], null, true)
    win.connect("destroy", dispose)
    win.present()
    run(win, start)
      .catch((error) => console.error("probe failed:", error))
      .finally(() => app.quit())
  })
})

await app.runAsync([])
