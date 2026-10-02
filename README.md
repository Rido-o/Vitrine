# Vitrine

[![Check](https://github.com/Rido-o/Vitrine/actions/workflows/check.yml/badge.svg)](https://github.com/Rido-o/Vitrine/actions/workflows/check.yml)

A GTK4 image viewer and wallpaper picker for Linux, written in Rust with
[gtk4-rs](https://gtk-rs.org). Browse a folder as a thumbnail grid, open images
full-screen with zoom and pan, and set any image as your wallpaper with a
command of your choice.

## Features

- **Thumbnail grid**, each thumbnail at its image's own shape in equal 16:9
  cells that grow with the window until another column fits, with an on-disk
  thumbnail cache. Thumbnails are loaded or generated on worker threads (one
  per core but one, at low priority): the
  tiles you stop on come first, and ones scrolled past are skipped. Once a
  folder is scanned, its other missing thumbnails are generated in the
  background while no tile is waiting. The thumbnails of the tiles the grid
  has bound (the rows around the viewport, ~390) plus at most 100 more stay in
  memory, so memory stays bounded in huge folders. Images smaller than a
  thumbnail keep their size. Folders are scanned on a thread, so the window
  opens immediately and the grid fills in as images are found. It refreshes
  itself when images are added, removed, renamed or edited (file monitors on
  up to 1,000 folders, at most one rescan a second); changes made on another
  machine, e.g. directly on an NFS server, aren't seen, so press `r` (or use
  the ⋯ menu) for those.
- **Top bar**: a folder entry (`~` works) with a popover of the last 10
  folders; a sort pill with Name (full path), Date modified, Size and Random
  (click again to reshuffle), and ↑/↓ to flip the direction (Date and Size
  start descending); a Subfolders toggle; and, on the right, image properties
  (i), more actions (⋯) and close.
- **Info bar**: the filename (click to show it in your file manager over
  `org.freedesktop.FileManager1`), its resolution (as shown, after EXIF
  rotation), its file size and View.
- **Full-screen view**: scroll to zoom around the cursor (from fit up to 8×
  actual pixels), drag to pan, click the left/right edge (a sixth of the width)
  for the previous/next image, double-click (the middle) to toggle fit/100%,
  `s` for sharp (nearest-neighbour) pixels, `[`/`]` to rotate and `h`/`v` to
  flip (also in the ⋯ menu; only the view changes, never the file, and it
  resets for the next image), and a button (or `f`) to make the window
  fullscreen; leaving the view restores it. When fullscreen, the buttons, info
  and cursor fade out after 2 s without mouse movement (not while the pointer
  is on them or a menu is open). An image is decoded at the view's size in
  device pixels and drawn at exactly those pixels at fit; zooming past it
  decodes the full resolution into tiles, of which only the visible ones are
  drawn. The two images on each side are decoded in the background, so ←/→
  are instant, and the image selected in the grid is decoded before you open
  it. While an image is still decoding (e.g. holding an arrow key) its
  thumbnail is shown, then swapped for the full image. Nothing decodes on the
  main thread.
- **Colour assessment** (`b` in the full-screen view, or the ⋯ menu), after
  darktable's, along the lines of ISO 12646: the image on middle grey (L\* 50,
  sRGB 119,119,119) inside a white frame, a neutral surround for judging
  exposure and a white reference for contrast, instead of black. The border
  on each side is 20% of the view's shorter side, 40% of it white; the image
  is decoded for the area inside it, and when zoomed in it stays clipped
  within the frame. The buttons and info fade out as in fullscreen. It stays
  on for the window until it's closed.
- **Image properties** (the i button on either screen, or `i`): name, folder,
  size, type and dates; dimensions (as shown, after EXIF rotation), megapixels,
  format and orientation; and, when the file has EXIF, the camera, lens,
  exposure, aperture, ISO, focal length, flash, date taken, GPS location,
  software, artist and copyright (read with kamadak-exif, written as exiv2
  prints them). Values can be selected to copy them.
- **More actions** (the ⋯ button on either screen): Copy image (`Ctrl+C`; the
  full image, or a GIF's first frame), Copy path (`Ctrl+Shift+C`), Set as
  wallpaper (`w`, with a wallpaper command), Show in file manager, Rescan
  folder and Keyboard shortcuts (`?`, a window listing every key). On Wayland
  the clipboard is served by Vitrine, so a copy lasts only while it's open
  unless a clipboard manager keeps it.
- **Set as wallpaper** runs a configurable command with the image path and
  shows a "Wallpaper set" toast; the viewer stays open.
- **Delete** moves the image to the trash, and **Ctrl+Z** (or the toast's Undo)
  restores it, repeatedly back through every delete in the window. It's the
  desktop's own trash (the freedesktop.org one your file manager shows):
  the home trash, or a drive's own `.Trash-<uid>` for images on another
  drive; no GVfs needed.
- JPEG, PNG, WebP, TIFF and GIF (animated GIFs play in the full-screen view;
  thumbnails show the first frame); a desktop entry registers Vitrine for
  those types. Photos are shown upright: the EXIF orientation is applied, in
  any format that has one, and the sizes shown are the upright ones.
  Embedded colour profiles (Adobe RGB, Display P3, ProPhoto…) are converted to
  sRGB on the worker threads, for thumbnails and the full-screen view alike;
  images without one are taken to be sRGB.

## Usage

```sh
vitrine                  # opens ~/Pictures (or the current directory)
vitrine DIR              # grid on DIR
vitrine FILE             # FILE's folder, with FILE open in the full-screen view
vitrine -r DIR           # start with Subfolders on (--subfolders)
```

Keys (grid unless noted):

| Key | Does |
| --- | --- |
| Enter, `e`, double-click | Open in the full-screen view |
| `w` | Set as wallpaper, with a wallpaper command (grid and view) |
| `i` | Image properties (grid and view) |
| Ctrl+C, Ctrl+Shift+C | Copy the image, or its path (grid and view) |
| `?` | Keyboard shortcuts (grid and view) |
| `r` | Rescan (grid and view) |
| Delete | Move to trash (grid and view) |
| Ctrl+Z | Undo the last delete; repeat to go further back (grid and view) |
| Ctrl+W, Ctrl+Q | Close the window (grid and view) |
| Esc, `q` (view) | Back to the grid |
| ↓ (folder entry) | Open the folder history; Esc puts the entry back |
| ←/→ (view) | Previous/next image |
| click the left/right sixth (view, at fit) | Previous/next image; the cursor shows an arrow there |
| scroll, drag (view) | Zoom around the cursor, pan when zoomed in |
| double-click (view) | Toggle fit/100%: the middle at fit, anywhere when zoomed |
| `+`/`=`, `-`, `0` (view) | Zoom in, out, fit |
| `s` (view) | Sharp pixels |
| `[`, `]` (view) | Rotate left, right (only the view; resets for the next image) |
| `h`, `v` (view) | Flip horizontally, vertically (only the view) |
| `b` (view) | Colour assessment: grey surround, white frame |
| `f` (view) | Toggle fullscreen; leaving the view restores the window |

Files:

- Thumbnails: `~/.cache/vitrine/thumbnails-4`, fitting 440×320: JPEG, or PNG
  for images with transparent pixels, named after the image's path and
  modification time. A thumbnail is refreshed when it's used, and a few
  seconds after launch Vitrine deletes any not used for 90 days, so
  thumbnails of edited, moved or deleted images don't pile up (and any left
  half-written by a Vitrine that was killed, after an hour).
- Folder history: `~/.local/state/vitrine/history`.
- Your own styles (optional): `~/.config/vitrine/style.css`, see "Theming".

With NVIDIA's driver loaded, Vitrine uses GTK's GL renderer (see "Gotchas");
set `GSK_RENDERER` to override it (e.g. `GSK_RENDERER=vulkan`).

(Earlier versions' thumbnail caches, `~/.cache/vitrine/thumbnails`,
`~/.cache/vitrine/thumbnails-2`, `~/.cache/vitrine/thumbnails-3` (before
colour profiles were read), `~/.cache/vitrine-spike` and
`~/.cache/shard-view`, are deleted in the background. The Rust port's own
folder history, `~/.local/state/vitrine-spike/history`, is moved over once.)

## Installing

With Nix flakes:

```sh
nix run github:Rido-o/Vitrine -- DIR
```

In a Home Manager configuration:

```nix
{
  inputs.vitrine.url = "github:Rido-o/Vitrine";
  inputs.vitrine.inputs.nixpkgs.follows = "nixpkgs";
}
```

```nix
{inputs, ...}: {
  imports = [inputs.vitrine.homeManagerModules.default];
  programs.vitrine = {
    enable = true;
    wallpaperCommand = "swww img"; # anything that takes an image path
    defaultViewer = true;
  };
}
```

| Option | Does |
| --- | --- |
| `programs.vitrine.enable` | Installs Vitrine. |
| `programs.vitrine.package` | The package to use. |
| `programs.vitrine.wallpaperCommand` | Command run with the image path appended; sets `VITRINE_WALLPAPER_COMMAND` as a default in a wrapper. Null (the default) hides Set as wallpaper. |
| `programs.vitrine.defaultViewer` | Runs `xdg-mime default io.github.Rido_o.Vitrine.desktop` for JPEG, PNG, WebP, TIFF and GIF on each activation. `~/.config/mimeapps.list` stays unmanaged, so other defaults and your file manager's "Open With" keep working. |
| `programs.vitrine.style` | CSS written to `~/.config/vitrine/style.css` (see "Theming"). Empty (the default) leaves the file unmanaged. |

Without the module, set `VITRINE_WALLPAPER_COMMAND` yourself; it's parsed like
a shell command and the image path is appended.

### Theming

Vitrine is dark, and light when the desktop prefers light (the
`org.freedesktop.appearance` colour scheme; no preference stays dark). Its
colours are CSS variables (`--bg`, `--bg-surface`, `--bg-raised`,
`--bg-elevated`, `--fg`, `--fg-muted`, `--fg-bright`, `--border`,
`--border-solid`, `--separator`, `--accent`, `--accent-fg`, `--red`, `--hover`,
`--active`, `--shadow`; `style/theme.scss` has the palettes), so
`~/.config/vitrine/style.css`, loaded after the built-in styles, can change
them or any rule:

```css
:root { --accent: #d69094; }
@media (prefers-color-scheme: light) {
  :root { --accent: #a0525a; }
}
```

It's read at startup; parse errors are printed to stderr.

## Development

```sh
nix run . -- DIR                                   # build and run (-r for subfolders)
VITRINE_WALLPAPER_COMMAND=set-wallpaper nix run . -- DIR
nix flake check                                    # the build, clippy (-D warnings), rustfmt
nix develop                                        # cargo, clippy, rustfmt, rust-analyzer, dart-sass
nix fmt                                            # alejandra (Nix files)
```

In `nix develop`, `cargo build --release` and `cargo fmt` work as usual
(`build.rs` needs `sass` from the shell).

To try local changes in a NixOS/Home Manager config that uses the flake input,
override it with the checkout:
`nixos-rebuild switch --override-input vitrine path:$HOME/Projects/Vitrine`
(or `nh os switch -- --override-input …`).

CI (`.github/workflows/check.yml`) runs `nix flake check` and `nix build` on
every push to `master` and on pull requests.

`bench/` measures it in a headless sway (frame times, main-loop stalls,
thumbnail and decode times, memory) and has a UI check that drives the
controls; see `bench/README.md`. Quick check without opening a window:
`vitrine --help`.

### Layout

```
flake.nix           packages.default, homeManagerModules.default,
                    checks (clippy, rustfmt), devShell
package.nix         the build
hm-module.nix       programs.vitrine
Cargo.toml
build.rs            compiles style/style.scss into the binary
src/
  main.rs           Gtk.Application: CSS, icon path, command line, malloc and
                    renderer tuning
  window/           a window (`Window`: its state, building it, loading
                    folders), split by part:
    toolbar.rs        folder entry and history panel, sorting, Subfolders
    grid.rs           the thumbnail grid, its 16:9 cells, keeping the first
                      image selected while loading
    info.rs           the info bar, title and empty-folder message
    navigation.rs     opening, moving through and closing the full-screen view
    menu.rs           the ⋯ menu's actions, i buttons, copy, wallpaper
    delete.rs         trash and undo
    keys.rs           the window's keys
    toast.rs          brief messages over both pages
  library.rs        scanning (a thread), the sorted list model, rescans,
                    watching
  thumbnails/
    pool.rs           thumbnail workers
    cache.rs          the disk cache, pruning, old caches
    tiles.rs          the grid's thumbnail textures (bound tiles and 100 more)
  view/             the full-screen view: its page and controls (mod.rs)
    preview.rs        its decodes: ±2 preloads, full resolution tiles, GIF
                      frames, on worker threads
    zoomable.rs       the image widget (zoom, pan, rotate, flip, sharp mode,
                      tiles, colour assessment)
    autohide.rs       fading the controls when idle
  decode/           decoding (libjpeg-turbo scaled, png, GdkPixbuf), resizing
                    (Lanczos3), EXIF orientation, freeing big buffers off the
                    main thread (mod.rs)
    color.rs          embedded ICC profiles (JPEG, PNG, GdkPixbuf) converted
                      to sRGB (moxcms)
  desktop/          the file manager (D-Bus) and wallpaper command (mod.rs)
    trash.rs          trash and exact-item restore (the `trash` crate)
  actions.rs        the ⋯ menu: its model and window actions
  properties.rs     the i popover's contents (file info, GdkPixbuf, EXIF)
  shortcuts.rs      the keyboard shortcuts window
  history.rs        folder history file
  probe/            VITRINE_PROBE: hooks and helpers (mod.rs), the benchmark
                    (bench.rs) and the UI check (ui.rs)
style/
  style.scss        styles
  theme.scss        colour palette
icons/              bundled symbolic icons
bench/              benchmark harness, corpus script, results
```

- App ID `io.github.Rido_o.Vitrine`. The app is `NON_UNIQUE`: every launch is
  its own process and window; nothing stays running in the background.
- All GTK work is on the main thread; worker threads only produce pixels,
  wrapped in a `gdk::MemoryTexture` on the main thread without a copy.
- A window's state (`Window`) is an `Rc` its signal handlers hold weakly; the
  window's `destroy` handler holds the one strong reference.

### Build

`package.nix` builds the crate with `rustPlatform.buildRustPackage`
(`build.rs` compiles `style/style.scss` with dart-sass), installs the icons and
the desktop entry, and wraps `bin/vitrine` with `wrapGAppsHook4`'s arguments
by hand (`dontWrapGApps`) so its own GdkPixbuf `loaders.cache` (gdk-pixbuf's
loaders plus librsvg and `webp-pixbuf-loader`) overrides the hook's, with
`shared-mime-info` after the session's own data directories.

### Gotchas

Things that broke and look like harmless cleanups:

- **Use GTK's GL renderer with NVIDIA's driver** (`prefer_gl_on_nvidia` in
  `main.rs`, when `/proc/driver/nvidia` exists and `GSK_RENDERER` isn't set).
  With GTK's default Vulkan renderer, NVIDIA's driver costs several ms of
  main-thread time per new texture (`ioctl`s in `perf`/`strace`): about 12 ms
  per row of new thumbnails while scrolling, 100–300 ms when a screenful
  arrives at once, and 76 ms frames panning at 100% (each tile panned into view
  is a new texture). It's a known driver problem; other GPUs keep GTK's
  default.
- **Draw the view with a plain texture node** (`append_texture` in
  `view/zoomable.rs`). A scaled texture node with the linear filter came out
  blurred at display scales above 1 even when drawn 1:1 (58% of the texture's
  edge contrast at 1.5×); the plain node keeps all of it. Only sharp mode uses
  a scaled node (nearest).
- **Decode for the view at its size in device pixels and draw 1:1 at fit**
  (`view_size` in `window/navigation.rs`, `layout` in `view/zoomable.rs`).
  Decoding for the monitor and drawing it scaled to the window softened every
  image; the texture is resized with Lanczos3, and decoded again when the
  view's size changes, and for the area inside the colour assessment border
  (`zoomable::image_area`, used by both).
- **Never evict the thumbnails of bound tiles** (`thumbnails/tiles.rs`). The
  grid keeps ~390 tiles bound (rows around the viewport, not only the visible
  ones); with a cap of 300 on every texture, off-screen tiles evicted visible
  ones, and the full-screen view opened without its placeholder. Only textures
  of tiles no longer bound are capped (100).
- **Keep `tune_malloc`** (`main.rs`). glibc raises its mmap threshold each
  time a large block is freed, so the workers' multi-MB decode buffers ended
  up in per-thread arenas that never shrink (~230 MB after generating a
  3,000-image folder); a fixed 1 MB threshold and 4 arenas cut the peak by
  ~120 MB with no measurable slowdown.
- **Free big buffers off the main thread** (the janitor in `decode/mod.rs`).
  Dropping a 33 MB decode on the main thread took 0.7–4 ms per selection
  change, enough to delay the selection's highlight a frame at 144 Hz.
- **While a folder loads, keep the first image selected and the grid at the
  top** (`keep_first_while_loading` in `window/grid.rs`), until the user
  clicks, types or scrolls in the grid. Batches arrive in any order and are
  sorted as they come; the automatic selection stayed on whichever image
  arrived first, and GTK kept it in view (a 7,756-image folder opened halfway
  down).
- **Re-sort at once, not incrementally** (`resort` in `library.rs`), so the
  selected image can be put back straight after (~5–13 ms for 3,000 images).
  Loading still sorts incrementally, so a big batch doesn't block a frame.
- **A rescan re-shows the view's image only if it changed or went**
  (`finished` in `window/mod.rs`): rescans start on their own when anything
  in the folder changes, and re-showing resets the zoom.
- **The selection is a ring that fades in while settling onto the thumbnail**
  (160 ms, `outline-color` and `outline-offset` in `style.scss`): the default
  theme's animated highlight looked choppy next to 144 Hz scrolling, and none
  at all felt abrupt. Only the outline changes, so it stays cheap.
- **WebP thumbnails need the package's own `loaders.cache`.** `wrapGAppsHook4`
  sets `GDK_PIXBUF_MODULE_FILE` to librsvg's cache, which has no WebP. Adding
  a `--set` to `gappsWrapperArgs` isn't enough: the hook's comes later and
  wins, hence the manual wrap.
- **Undo restores an exact trash item, never "the newest".** The trash's
  deletion dates have one-second resolution, so two deletes of the same path
  in a second can't be told apart by date. `desktop/trash.rs` lists the
  path's trash items just before and after trashing; the one new item is
  recorded and undo moves exactly that back.
- **Restoring cleans up after the `trash` crate.** It puts an empty file at
  the old path first, then renames the item over it: a rename that fails
  (e.g. across drives, for an image from a drive without a trash of its own,
  kept in the home trash) left that empty file where the image was. It's
  removed, and an item on another drive is copied back instead.
- **Testing trash code needs its own `XDG_DATA_HOME`, on the same filesystem
  as the files it trashes**, or it trashes into your real trash, or that
  filesystem's own (e.g. `/tmp/.Trash-<uid>`); the UI check only trashes with
  `VITRINE_PROBE_TRASH=1` (see `bench/README.md`).
- **No transition between the grid and the full-screen view.** A 150 ms
  crossfade rendered cleanly but still looked laggy, so the switch is instant.
- **GTK uploads textures on the main thread when first drawn**, with no API to
  do it elsewhere: the frame that first shows a 4K image still spends ~55 ms
  uploading it. Hence screen-size textures, and tiles uploaded at most 3 per
  frame.

## Roadmap

- Fix Ctrl+C in the properties popover: with a value selected, nothing is
  copied (neither the text nor the image). The values aren't focusable, so
  the label never gets the key, and a key controller on the popover didn't
  see it either. In the headless check, `wtype`'s keys didn't reach the
  window at that point, so it's still unclear where the key goes; find that
  first.
- Maybe: uploading textures off the main thread, if screen-size textures and
  tiles stop being enough (a shared GL context, or dmabufs filled on a
  worker; both fragile).

## History

Vitrine started as the wallpaper picker in the author's
[AGS](https://github.com/aylur/ags) desktop shell, was ported to a standalone
GJS app called shard-view, renamed and moved to its own repo, and then
rewritten in Rust with gtk4-rs after a performance spike: scrolling, opening
images and memory all came out ahead of the TypeScript version
(`bench/RESULTS.md` has the comparison).

## Credits

- Icons: [Font Awesome Free](https://fontawesome.com) 7.3.1, licensed
  [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
- [gtk4-rs](https://gtk-rs.org) (MIT),
  [turbojpeg](https://crates.io/crates/turbojpeg) (MIT or Unlicense),
  [fast_image_resize](https://crates.io/crates/fast_image_resize) and
  [png](https://crates.io/crates/png) (MIT or Apache-2.0),
  [kamadak-exif](https://crates.io/crates/kamadak-exif) (BSD-2-Clause),
  [moxcms](https://crates.io/crates/moxcms) (BSD-3-Clause or Apache-2.0), and
  [trash](https://crates.io/crates/trash) (MIT).

## Licence

Not decided yet. Until a `LICENSE` file is added, all rights are reserved.
