# Vitrine

[![Check](https://github.com/Rido-o/Vitrine/actions/workflows/check.yml/badge.svg)](https://github.com/Rido-o/Vitrine/actions/workflows/check.yml)

A GTK4 image viewer and wallpaper picker for Linux, written in TypeScript for
GJS with [gnim](https://github.com/aylur/gnim) for JSX. Browse a folder as a
thumbnail grid, open images full-screen with zoom and pan, and set any image as
your wallpaper with a command of your choice.

## Features

- **Thumbnail grid** with an on-disk thumbnail cache and lazy,
  concurrency-limited loading; at most 300 decoded thumbnails stay in memory,
  so memory stays bounded in huge folders. Folders are scanned in the
  background (4 folders at a time), so the window opens immediately and the
  grid fills in as images are found, even for large folders over NFS. It
  refreshes itself when images are added, removed, renamed or edited (Gio file
  monitors on up to 1,000 folders, at most one rescan a second); changes made
  on another machine, e.g. directly on an NFS server, aren't seen, so press
  `r` for those.
- **Top bar**: a folder entry (`~` works) with a history of the last 10
  folders; a sort pill with Name (full path), Date modified, Size and Random
  (click again to reshuffle), and ↑/↓ to flip the direction (Date and Size
  start descending); a Subfolders toggle; and, on the right, image properties
  (i), more actions (⋯) and close.
- **Info bar**: the filename (click to show it in your file manager over
  `org.freedesktop.FileManager1`), resolution, rescan, view and Set wallpaper.
- **Full-screen view**: scroll to zoom around the cursor (from fit up to 8×
  actual pixels), drag to pan, click the left/right edge (a sixth of the width)
  for the previous/next image, double-click (the middle) to toggle fit/100%,
  `s` for sharp (nearest-neighbour) pixels, and a button (or `f`) to make the
  window fullscreen; leaving the view restores it. When fullscreen, the buttons,
  info and cursor fade out after 2 s without mouse movement (not while the
  pointer is on them or a menu is open). Images are drawn with GTK's
  default filter, snapped to whole device pixels. The two images on each side are decoded in
  the background, so ←/→ are instant, and decoding never freezes the window.
  While an image is still decoding (e.g. holding an arrow key) its thumbnail
  is shown, then swapped for the full image.
- **Image properties** (the i button on either screen, or `i`): name, folder,
  size, type and dates; dimensions (as shown, after EXIF rotation), megapixels,
  format and orientation; and, when the file has EXIF, the camera, lens,
  exposure, aperture, ISO, focal length, flash, date taken, GPS location,
  software, artist and copyright (read with gexiv2). Values can be selected
  to copy them.
- **More actions** (the ⋯ button on either screen): Copy image (`Ctrl+C`; the
  full image, or a GIF's first frame), Copy path (`Ctrl+Shift+C`), Show in
  file manager, Rescan folder and Keyboard shortcuts (`?`, a window listing
  every key). On Wayland the clipboard is served by Vitrine, so a copy lasts
  only while it's open unless a clipboard manager keeps it.
- **Set wallpaper** (button or `w`) runs a configurable command with the image
  path and shows a "Wallpaper set" toast; the viewer stays open.
- **Delete** moves the image to the trash, and **Ctrl+Z** (or the toast's Undo)
  restores it, repeatedly back through every delete in the window. Both need
  [GVfs](https://gitlab.gnome.org/GNOME/gvfs) (its `trash:///`), which most
  full desktops run; without it they're disabled.
- JPEG, PNG, WebP, TIFF and GIF (animated GIFs play in the full-screen view;
  thumbnails show the first frame); a desktop entry registers Vitrine for
  those types.
  Photos are shown upright: the EXIF orientation is applied.

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
| Enter, double-click | Open in the full-screen view |
| `w` | Set as wallpaper (grid and view) |
| `i` | Image properties (grid and view) |
| Ctrl+C, Ctrl+Shift+C | Copy the image, or its path (grid and view) |
| `?` | Keyboard shortcuts (grid and view) |
| `r` | Rescan |
| Delete | Move to trash (needs GVfs) |
| Ctrl+Z | Undo the last delete; repeat to go further back (grid and view) |
| Esc, `q` | Close the view, or quit from the grid |
| ←/→ (view) | Previous/next image |
| click the left/right sixth (view, at fit) | Previous/next image; the cursor shows an arrow there |
| scroll, drag (view) | Zoom around the cursor, pan when zoomed in |
| double-click (view) | Toggle fit/100%: the middle at fit, anywhere when zoomed |
| `+`/`=`, `-`, `0` (view) | Zoom in, out, fit |
| `s` (view) | Sharp pixels |
| `f` (view) | Toggle fullscreen; leaving the view restores the window |

Files:

- Thumbnails: `~/.cache/vitrine/thumbnails-2`. A thumbnail is refreshed when it's
  used, and a few seconds after launch Vitrine deletes any not used for 90
  days, so thumbnails of edited, moved or deleted images don't pile up.
- Folder history: `~/.local/state/vitrine/history`

(The shard-view era's folder history is moved over once. Older thumbnail
caches, `~/.cache/vitrine/thumbnails` and `~/.cache/shard-view`, held unrotated
thumbnails and are deleted in the background.)

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
| `programs.vitrine.wallpaperCommand` | Command run with the image path appended; sets `VITRINE_WALLPAPER_COMMAND` as a default in a wrapper. Null (the default) hides the Set wallpaper action. |
| `programs.vitrine.defaultViewer` | Runs `xdg-mime default io.github.Rido_o.Vitrine.desktop` for JPEG, PNG, WebP, TIFF and GIF on each activation. `~/.config/mimeapps.list` stays unmanaged, so other defaults and your file manager's "Open With" keep working. |

Without the module, set `VITRINE_WALLPAPER_COMMAND` yourself; it's parsed like
a shell command and the image path is appended.

## Development

```sh
nix run . -- DIR                                   # build and run
VITRINE_WALLPAPER_COMMAND=set-wallpaper nix run . -- DIR
nix flake check                                    # type check (tsc) and flake checks
nix develop                                        # gjs, esbuild, dart-sass, tsc; links node_modules
nix fmt                                            # alejandra
```

To try local changes in a NixOS/Home Manager config that uses the flake input,
override it with the checkout:
`nixos-rebuild switch --override-input vitrine path:$HOME/Projects/Vitrine`
(or `nh os switch -- --override-input …`).

CI (`.github/workflows/check.yml`) runs `nix flake check` and `nix build` on
every push to `master` and on pull requests.

Quick check without opening a window: `vitrine --help` loads the whole bundle
before GApplication prints its help.

### Layout

```
flake.nix           packages.default, homeManagerModules.default,
                    checks.typecheck, devShell
package.json        typescript and @girs types, for type checking only
tsconfig.json
package.nix         the build
hm-module.nix       programs.vitrine
src/
  main.tsx          Gtk.Application: CSS, icon path, command line, windows
  jsx.ts            registers lowercase JSX tags (box, entry, …) with gnim
  Window.tsx        layout: top bar, grid, info bar, full-screen view
  Library.ts        async folder scanning, sorting, list model, mtime/size
  Thumbnails.ts     thumbnail cache (disk + memory), loading, concurrency
  History.ts        folder history file
  ZoomableImage.ts  full-screen image widget (zoom, pan, sharp mode)
  Properties.ts     the properties panel's contents (file info, GdkPixbuf, EXIF)
  Shortcuts.ts      the keyboard shortcuts window
  ImageCache.ts     full-size images: the one shown plus ±2 preloaded
  decode.ts         threaded image and GIF decoding (GdkPixbuf), shared with
                    thumbnails
  Trash.ts          trash and exact-item restore through GVfs (trash:///)
  util.ts           names, folder helpers, file-manager D-Bus call, wallpaper
  style.scss        styles
  theme.scss        colour palette
  env.d.ts          gi:// module and GJS global types (@girs), ICONS_DIR
  assets.d.ts       the bundled CSS module
icons/              bundled symbolic icons
```

- App ID `io.github.Rido_o.Vitrine`. The app is `NON_UNIQUE`: every launch is
  its own process and window; nothing stays running in the background.
- Everything is plain GTK4/Gio/GdkPixbuf/GLib, plus gexiv2 for EXIF; the only
  JS library is gnim.

### Build

`package.nix`:

1. Copies the `gnim` input to `node_modules/gnim` and links `dist` to its `src`
   (gnim's `package.json` exports `./dist`, which its own build copies from
   `./src`).
2. Compiles `src/style.scss` with dart-sass.
3. Bundles `src/main.tsx` with esbuild: ESM, `gi://*` and GJS built-ins
   external, JSX via `gnim/gtk4`, CSS loaded as text, `ICONS_DIR` defined as
   the installed icons path.
4. Installs `share/vitrine/{main.js,icons}`, the desktop entry and
   `bin/vitrine` (`gjs -m main.js`), wrapped with `wrapGAppsHook4`'s arguments
   by hand (`dontWrapGApps`) so its own GdkPixbuf `loaders.cache` (gdk-pixbuf's
   loaders plus librsvg and `webp-pixbuf-loader`) overrides the hook's.
   `gexiv2_0_16` (EXIF) is a build input so the wrapper puts its typelib on
   `GI_TYPELIB_PATH`.

esbuild strips types without checking them; `checks.typecheck` (run by
`nix flake check`) does, with `tsc --noEmit` against the `@girs` type packages
pinned in `package-lock.json` (installed offline with `importNpmLock`). They're
pinned to the `4.0.0-rc.17` generation, which covers GTK 4.23 and matches what
gnim is written against; the 5.x packages type signal names more strictly than
gnim's code allows. gnim's own `.ts` source is marked `@ts-nocheck` in the
check (`skipLibCheck` only covers `.d.ts`), so only Vitrine is checked.

### Gotchas

Things that broke and look like harmless cleanups:

- **`await app.runAsync(…)`, not `app.run(…)`.** A blocking `run()` at module
  top level stops GJS from resolving promises while the main loop runs:
  thumbnails loaded but their `.then()` never ran, leaving blank tiles.
- **Each window is created inside gnim's `createRoot`** (`main.tsx`), disposed
  on `destroy`; without it gnim logs "out of tracking context" and can't clean
  up.
- **WebP thumbnails need the package's own `loaders.cache`.** `wrapGAppsHook4`
  sets `GDK_PIXBUF_MODULE_FILE` to librsvg's cache, which has no WebP, so
  GdkPixbuf (thumbnails, resolution) failed on `.webp` while the full-screen
  view (GTK's own loaders) worked. Adding a `--set` to `gappsWrapperArgs`
  isn't enough: the hook's comes later and wins, hence the manual wrap.
- **While a scan is running, keep the auto-selection on the first image**
  (`onLibraryChanged` in `Window.tsx`): batches insert images ahead of it, and
  GTK would otherwise keep it selected and scroll the grid down after it.
- **Keep the `System.gc()` nudge in `Thumbnails.ts`.** At most 300 thumbnails
  are kept in memory, but dropped textures (and every decode's pixbuf) are only
  freed when GJS collects their wrappers, and its GC doesn't see their native
  memory. Without the nudge, memory kept growing past the cap: 526 MB after
  1,000 cached thumbnails versus a flat ~230 MB with it (~405 MB once
  full-size originals are being decoded, a high-water mark that then stays
  flat). Generating thumbnails needs its own nudge (every 5): each decode of a
  large image kept ~8 MB alive until collected, so generating 348 screenshot
  thumbnails peaked at 2.9 GB, versus ~0.35 GB with it and no slower; a folder
  under ~400 images never evicts, so the eviction nudge alone never ran.
- **~800 MB after scrolling a large folder isn't a leak.** After a warm scroll
  through 7,756 images, `/proc/PID/smaps` showed ~550 MB of `[heap]` and ~200
  MB of GPU driver mappings (`/dev/nvidiactl`), with only ~170 MB of
  thumbnails live. The heap is freed render allocations that glibc keeps: once
  large buffers are freed it raises its mmap threshold, so later ~0.5 MB
  texture allocations come from the heap, and fragmentation stops it
  shrinking. It's the same in a gtk4-rs prototype and with every GSK renderer,
  and it stays bounded (444–904 MB over five runs). `GLIBC_TUNABLES=
  glibc.malloc.mmap_threshold=131072` brings it to ~470 MB, but it isn't set:
  thumbnail generation got ~6% slower, and every texture allocation would
  mmap on the main thread; frame times weren't measured, and responsiveness
  matters more than memory here.
- **Decode with GdkPixbuf's async API (`decode.ts`), not
  `Gdk.Texture.new_from_bytes`.** The latter decodes on the main thread and
  froze the window for the whole decode (up to ~150 ms for a 12 MP WebP, ~90
  ms for 8K JPEGs); preloading four neighbours with it would freeze on every
  keypress. The async API takes the same time in a worker thread (measured
  stalls ≤ 12 ms). With preloading, the next image shows in ~0 ms instead of
  ~70 ms.
- **Preloads are cancelled and queued (`ImageCache.ts`).** Dropping a preload
  from the cache must cancel its decode, and at most two decodes run with the
  shown image first: otherwise holding an arrow key (~30 presses/s) piled up
  decodes of images already passed, and the one stopped on appeared ~925 ms
  after release instead of ~40 ms.
- **Copy each GIF frame (`copyToTexture` in `decode.ts`).** GdkPixbuf's
  animation iterator draws every frame into the same pixbuf, and
  `Gdk.Texture.new_for_pixbuf` shares its pixels, so a texture made from it
  would change under GTK. Frames are played live on the widget's frame clock
  (GdkPixbuf can't say how many frames there are), each as a new texture,
  with a GC nudge every 64 MB of them.
- **Undo restores an exact trash item, never "the newest".** GVfs's deletion
  dates have one-second resolution, so two deletes of the same path in a
  second can't be told apart by date (restoring the "newest" picked the wrong
  one in testing). `Trash.ts` lists the path's `trash:///` items just before
  and after trashing; the one new item is recorded and undo moves exactly that
  back.
- **Testing trash code headless needs GVfs on a private bus**:
  `dbus-run-session`, `GIO_EXTRA_MODULES=<gvfs>/lib/gio/modules` and
  `<gvfs>/libexec/gvfsd --replace &` before running the test.
- **Test decoding headless**: bundle a small entry that imports the module with
  esbuild and run it with the package's own `gjs` and `GI_TYPELIB_PATH` (from
  the wrapper); a different gjs mismatches the typelibs.

Scan timings (headless, recursive; "cold" is the first run after NFS attribute
caches expired):

| Folder | Images | Blocking scan | Async, first images | Async, complete |
| --- | --- | --- | --- | --- |
| Small | 53 | 16–45 ms | 2–3 ms | 10–11 ms |
| Large, over NFS | 9,680 | 0.45 s warm, 1.9 s cold | 2–3 ms | 0.24 s warm, ~1 s cold |

## Roadmap

- Show file size.
- More CSS improvements.
- Thumbnails at the image's own aspect ratio, and account for margins when
  calculating thumbnail width (they end up too wide).
- The folder history panel and sort pill could become a `Gtk.Popover` /
  `Gtk.DropDown` (they're inline because Vitrine started as a layer-shell
  overlay, where Hyprland dismissed popups on click).
- A colour theme that isn't hardcoded (`src/theme.scss`).

## History

Vitrine started as the wallpaper picker in the author's
[AGS](https://github.com/aylur/ags) desktop shell, was ported to a standalone
GJS app called shard-view, and was renamed and moved to its own repo.

## Credits

- Icons: [Font Awesome Free](https://fontawesome.com) 7.3.1, licensed
  [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
- [gnim](https://github.com/aylur/gnim) (MIT) for JSX on GJS.

## Licence

Not decided yet. Until a `LICENSE` file is added, all rights are reserved.
