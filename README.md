# shard-view

A GTK4 image viewer written in TypeScript for GJS, using
[gnim](https://github.com/aylur/gnim) for JSX. It grew out of the wallpaper
picker in `ags-shell` and is being turned into a general viewer that doesn't
depend on AGS: a thumbnail grid for browsing a folder, a full-screen view with
zoom and pan, and a button to set the current image as the wallpaper.

It lives in this repo for now but is meant to be spun off into its own; see
[Spinning off](#spinning-off).

## Status

| Phase | State |
| --- | --- |
| 1. Scaffold | Done: builds, opens a placeholder window |
| 2. Port the picker | Done: grid, sorting, history, subfolders, view, set wallpaper |
| 3. Startup speed | Done: async scanning, cache moved to ~/.cache/shard-view |
| 4. Integration | Next |
| 5. Remove the ags picker | Planned |

## Goals

- A general image viewer: open a folder to browse it, or a file to see it in
  its folder.
- Everything the ags picker does, without a background process: each launch is
  a fresh process that exits when the window closes.
- A normal application window (not a layer-shell overlay), so standard GTK
  popovers and dropdowns work.
- No hard dependency on this repo's setup (see [Spinning off](#spinning-off)).

## Features

Carried over from the ags picker:

- Thumbnail grid with an on-disk thumbnail cache and lazy, concurrency-limited
  loading.
- Top bar: folder entry (`~` works) with a history of the last 10 folders
  (`~/.local/state/…/history`), and a sort pill: Name (full path), Date
  modified, Size, Random (click again to reshuffle), ↑/↓ direction; Date and
  Size start descending. The sort resets every launch.
- Info bar: filename (click to show in the file manager over D-Bus
  `org.freedesktop.FileManager1`), resolution, rescan, view.
- Full-screen view (`ZoomableImage`): scroll to zoom around the cursor (fit to
  8× actual pixels), drag to pan (until an image edge reaches the middle of the
  screen), double-click toggles fit/100%, `+`/`-`/`0` zoom keys, `s` toggles
  sharp (nearest-neighbour) pixels, ←/→ previous/next. The image is drawn with
  GTK's default filter and snapped to whole device pixels.
- Delete moves the image to the trash.

New:

- Command line: `shard-view [DIR | FILE]`.
- An "Include subfolders" toggle in the top bar, off by default (the ags picker
  was always recursive).
- A "Set wallpaper" button and key, available at all times; setting the
  wallpaper doesn't close the viewer, it confirms in the info bar.
- Registered as the default viewer for `image/*`.

Not planned yet (carried over from the picker's TODO list): multi-select to
cycle wallpapers, showing file size, click the left/right thirds of the view to
go previous/next, thumbnails at the image's own aspect ratio.

## Usage (target)

```sh
shard-view               # opens ~/Pictures (or the current directory)
shard-view DIR           # grid on DIR
shard-view FILE          # FILE's folder, with FILE open in the full-screen view
```

Keys (target; grid unless noted):

| Key | Does |
| --- | --- |
| Enter, double-click | Open in the full-screen view |
| `w` | Set as wallpaper (grid and view) |
| `r` | Rescan |
| Delete | Move to trash |
| Esc, `q` | Close the view, or quit from the grid |
| ←/→ (view) | Previous/next image |
| scroll, drag, double-click (view) | Zoom, pan, fit/100% |
| `+`/`=`, `-`, `0` (view) | Zoom in, out, fit |
| `s` (view) | Sharp pixels |

## Architecture

```
shard-view.nix      package (perSystem) + Home Manager module (this repo only)
src/
  main.tsx          Gtk.Application: CSS, icon path, command line, windows
  jsx.ts            registers lowercase JSX tags (box, entry, …) with gnim
  Window.tsx        layout: top bar, grid, info bar, full-screen view
  Library.ts        folder scanning, sorting, list model, mtime/size caches
  Thumbnails.ts     thumbnail cache (disk + memory), loading, concurrency
  History.ts        folder history file
  ZoomableImage.ts  full-screen image widget (zoom, pan, sharp mode)
  util.ts           folder helpers, file-manager D-Bus call, wallpaper command
  style.scss        styles; theme.scss is copied in at build time
icons/              bundled symbolic icons (Font Awesome Free, CC BY 4.0)
```

- The app is `NON_UNIQUE`: every launch is its own process and window.
- App ID `dev.shard.View` (used by the desktop entry and Hyprland window rules).
- Everything is plain GTK4/Gio/GdkPixbuf/GLib; the only library is gnim.

### Build

`nix build .#shard-view` (see `shard-view.nix`):

1. Copies the `gnim` flake input to `node_modules/gnim` and links `dist` to its
   `src` (gnim's `package.json` exports `./dist`, which its own build copies
   from `./src`).
2. Compiles `src/style.scss` with dart-sass (after copying `theme.scss` in).
3. Bundles `src/main.tsx` with esbuild: ESM, `gi://*` and GJS built-ins
   external, JSX via `gnim/gtk4`, CSS loaded as text, `ICONS_DIR` defined as
   the installed icons path.
4. Installs `share/shard-view/{main.js,icons}` and `bin/shard-view`
   (`gjs -m main.js`), wrapped with `wrapGAppsHook4`'s arguments by hand
   (`dontWrapGApps`) so our own GdkPixbuf `loaders.cache` (gdk-pixbuf's
   loaders plus librsvg and `webp-pixbuf-loader`, built with
   `gdk-pixbuf-query-loaders`) overrides the hook's.

There's no type checking: esbuild strips types. Adding `tsc --noEmit` with
`@girs` types is a spin-off task.

Quick check without opening a window: `shard-view --help` loads the whole
bundle before GApplication prints its help.

For a quick run without installing:
`SHARD_VIEW_WALLPAPER_COMMAND=set-wallpaper nix run .#shard-view -- DIR`.

### Gotchas

Things that broke during the port and look like harmless cleanups:

- **`await app.runAsync(…)`, not `app.run(…)`.** A blocking `run()` at module
  top level stops GJS from resolving promises while the main loop runs:
  thumbnails loaded but their `.then()` never ran, leaving blank tiles.
- **Each window is created inside gnim's `createRoot`** (`main.tsx`), disposed
  on `destroy`. AGS's `app.start()` did this implicitly; without it gnim logs
  "out of tracking context" and can't clean up.
- **WebP thumbnails need our own `loaders.cache`.** `wrapGAppsHook4` sets
  `GDK_PIXBUF_MODULE_FILE` to librsvg's cache, which has no WebP, so GdkPixbuf
  (thumbnails, resolution) failed on `.webp` while the full-screen view (GTK's
  own loaders) worked. Adding our `--set` to `gappsWrapperArgs` isn't enough:
  the hook's comes later and wins, hence the manual wrap.
- **Test decoding headless**: bundle a small entry that imports the module
  with esbuild and run it with the package's own `gjs` and `GI_TYPELIB_PATH`
  (from the wrapper); a different gjs mismatches the typelibs.

## Plan

### Phase 1: scaffold (done)

- `gnim` flake input, the package and Home Manager module, `main.tsx` with a
  placeholder window, `jsx.ts`, `style.scss`, one icon.

### Phase 2: port the picker (done)

Source: `modules/desktop/ags-shell/config/widget/wallpapers/` and
`styles/Wallpapers.scss`.

- Split `Wallpapers.tsx` into `Library.ts`, `Thumbnails.ts`, `History.ts` and
  `Window.tsx`; move `ZoomableImage.ts` over (imports only).
- Replace `Astal.Window` (fullscreen layer surface, exclusive keyboard, click
  outside to close) with a `Gtk.ApplicationWindow`; the grid box becomes the
  window content. Drop the click-outside handler.
- `ags/gtk4` imports become `gi://` imports; `app` becomes the
  `Gtk.Application`.
- Command line: `HANDLES_COMMAND_LINE` (or `HANDLES_OPEN`) for `DIR`/`FILE`;
  a file opens its folder with the view on that file.
- "Include subfolders" toggle (off by default); the scan takes a `recursive`
  flag, and toggling rescans.
- "Set wallpaper": button in the info bar and the preview, key `w`; runs the
  wallpaper command and shows a confirmation in the info bar. It doesn't close
  the window. The button is hidden when no command is configured (see
  [Configuration](#configuration)).
- Esc/`q` in the grid quits (it used to hide the overlay).
- Copy the icons the picker uses (folder, chevron-down, arrows, arrows-rotate,
  xmark, image) into `icons/`.
- Port the styles; the overlay window's transparent background and outer box
  shadow go away.
- Keep the history panel and sort pill as they are; they could become a
  `Gtk.Popover`/`Gtk.DropDown` later now that popups work.
- Set the program name (`GLib.set_prgname`) so it isn't reported as `gjs`.

### Phase 3: startup speed (done)

Every launch is cold now, and big folders live on NFS (`/mnt/data`).

- Asynchronous, batched scanning (`enumerate_children_async` /
  `next_files_async`, 200 entries per batch, 4 folders read concurrently since
  each call is an NFS round trip). The window shows straight away; images are
  inserted in sort order as batches arrive; opening another folder cancels the
  scan; the grid says "Scanning…" until the first images arrive.
- A file on the command line opens in the full-screen view before the scan;
  the grid selects it when the scan reaches it.
- The thumbnail cache moved from `~/.cache/ags/wallpapers` to
  `~/.cache/shard-view/thumbnails` (a one-off rename on first run, same file
  naming, so existing thumbnails are reused).
- Thumbnails stay JPEG; the freedesktop thumbnail spec (shared with Thunar) is
  an option later, but PNG decodes slower.

Scan timings (headless, recursive; "cold" is the first run after NFS
attribute caches expired, not a guaranteed-cold cache):

| Folder | Images | Blocking scan | Async, first images | Async, complete |
| --- | --- | --- | --- | --- |
| Desktop Wallpapers | 53 | 16–45 ms | 2–3 ms | 10–11 ms |
| `/mnt/data/Images` | 9,680 | 451 ms warm, 1.9 s cold | 2–3 ms | 244 ms warm, ~1 s cold |

### Phase 4: integration (this repo)

- ags-shell's bar button runs `shard-view <wallpaper folder>` instead of
  showing the `wallpapers` window.
- Hyprland window rule for `dev.shard.View` (float, ~85%×80%, centred) in
  `hyprland.lua`.
- `.desktop` entry (`Exec=shard-view %f`, `MimeType=image/*` list) and
  `xdg.mimeApps` defaults so it's the default image viewer.
- Stop importing `homeManager.nsxiv` on rei; keep the module in the repo.
- Import `homeManager.shard-view` on rei.

### Phase 5: remove the ags picker

- Delete `widget/wallpapers/`, `styles/Wallpapers.scss`, the icons only it used,
  and its registration in `app.tsx`.
- Remove the wallpaper picker section from `docs/ags-shell.md`; move the
  picker's items in `docs/TODO.md` here (see [Features](#features)).
- Add `docs/shard-view.md` (a pointer to this README, per the repo's docs
  convention) and list it in `docs/README.md`.

## Configuration

Planned, to keep repo-specific values out of the code:

- Wallpaper command: read from the `SHARD_VIEW_WALLPAPER_COMMAND` environment
  variable (the file path is appended as an argument). The Home Manager module
  sets it (here, to `set-wallpaper` from `modules/desktop/awww.nix`). Unset
  means no "Set wallpaper" button.
- Default folder: the command-line argument; the bar button passes the
  wallpaper folder.

## Spinning off

What ties it to this repo, and what to do about each when it moves:

| Coupling | Now | In its own repo |
| --- | --- | --- |
| Colours | `theme.scss` copied from `ags-shell` at build time | Vendor the palette, or read colours from a CSS file / GTK theme |
| Package | `perSystem` in this flake, `inputs.gnim` | Its own `flake.nix` with a `gnim` input and `packages.default`; this repo takes it as an input |
| Home Manager module | `flake.modules.homeManager.shard-view` here | Export `homeManagerModules.default` from the new flake, with options for the wallpaper command and default-viewer registration |
| Wallpaper command | `set-wallpaper` (awww) | Configuration only (see above); nothing hardcoded |
| Default folder | wallpaper folder passed by ags-shell | Command-line argument only |
| Docs | `docs/shard-view.md` points here | This README is the repo README |

Also worth doing then: `tsc --noEmit` with `@girs` types, a `LICENSE`, and
crediting Font Awesome for the icons.
