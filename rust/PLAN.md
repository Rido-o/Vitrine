# gtk4-rs spike

A minimal gtk4-rs version of Vitrine, measured against the TypeScript app
before any other feature is ported. Lives on the `rust-spike` branch; the
TypeScript app stays buildable next to it (`nix build`, `.#spike`,
`.#bench-ts`).

## Goal and decision rule

Same machine, corpus and metrics for both apps (`bench/`). Continue with a
full port only if the spike is clearly better on scrolling smoothness and
memory, and at least as good on the full-screen view.

## Scope

In: recursive scan of a folder given on the command line; a scrollable grid
with generated, disk-cached thumbnails; a full-screen view (Enter or
double-click, Esc back, ←/→ with preloading).

Out until later: sorting options, history and the directory entry, trash and
undo, properties, wallpaper, menus and shortcuts, file watching,
zoom/pan/rotate, GIF playback, the Home Manager module, most styling.

Own app ID (`io.github.Rido_o.Vitrine.Spike`) and cache
(`~/.cache/vitrine-spike/`), so the real app's data is untouched.

## Principles

- All GTK on the main thread; workers only produce pixels (`Vec<u8>` or
  `glib::Bytes`), wrapped in a `gdk::MemoryTexture` on the main thread without
  a copy.
- Decode and upload only the pixels that are shown: thumbnails at tile size,
  the full-screen view at screen size (in device pixels), full resolution only
  when zooming past fit.
- GTK uploads textures on the main thread when first drawn (no API to do it
  elsewhere, nothing in GTK 4.20–4.24 changes that), so keep each frame's new
  texture data small.

## Phase 0: scaffolding and benchmark harness

- Crate `rust/` (`gtk4` 0.11; glycin crate 3.1.0 matches nixpkgs'
  glycin-loaders 2.1.5, added in Phase 3), `rust/package.nix` wrapped like the
  TypeScript app (WebP `loaders.cache`, glycin loaders, MIME database).
- Probes printing the same `RESULT` lines: `rust/src/probe.rs`
  (`VITRINE_PROBE=1`) and `bench/probe-ts.tsx` (`.#bench-ts`).
- `bench/make-corpus.sh` (~3,000 images, nested) and `bench/run.sh` (headless
  sway, per-app cache and state).
- Baseline numbers from the TypeScript app.
- Done when: `.#spike` opens an empty window, and the probe runs against both.

## Phase 1: scanning

- Walk the tree on a worker thread (`std::fs`, filter by extension), stream
  batches over a channel into a `gio::ListStore`, sorted by name.
- A parallel walker (`jwalk`) only if the numbers call for it.
- Measure: time to first batch, total, main-thread stalls while inserting.
- Done when: the corpus lists with no main-thread stall over 8 ms.
- **Result:** the walk takes ~7 ms for 3,000 images in 51 folders (page cache
  warm), so it's over before the window's first frame (~170 ms of GTK/Vulkan
  startup); batches of 512 insert in ~80 µs, except the first (~8.5 ms: the
  grid creating its first screen of tiles, a one-off). One thread is plenty;
  no parallel walker. Scrolling 3,000 label tiles: p50 16.6 / p99 18.1 ms,
  no dropped frames.

## Phase 2: grid and thumbnails

- `GridView` + `SignalListItemFactory`, `gtk::Picture` tiles (440×320 box).
- Scheduler as in the TypeScript app: newest request first, skip tiles no
  longer bound, background generation only while no tile is waiting.
- Worker pool of cores − 1 threads: decode at tile size, EXIF orientation,
  transparency check, write the disk cache, return pixels.
- Memory cache: 300 textures, LRU, freed on drop.
- **2a:** GdkPixbuf `from_file_at_scale` on the workers (same decoders as the
  TypeScript app; its JPEG loader already uses libjpeg-turbo's reduced-size
  DCT decode), the apples-to-apples baseline.
- **2b:** `turbojpeg` with a 1/2–1/8 scaling factor plus `fast_image_resize`
  (SIMD) for the final step; kept only if faster. Not `zune-jpeg`: no scaled
  decode yet (zune-image#434). Other formats stay on GdkPixbuf or `image`.
- **2c:** a variant using the freedesktop thumbnail cache
  (`~/.cache/thumbnails/x-large`, 512 px, MD5 of the file URI, PNG with
  `Thumb::URI`/`Thumb::MTime`/`Thumb::Size`), shared with file managers:
  folders already browsed in one show instantly. Costs PNG encode/decode
  (on workers). A decision for later, since it changes the app's cache.
- Fallback if the grid itself is the bottleneck: one custom widget drawing all
  visible thumbnails in `snapshot` instead of a widget per tile.
- Measure: frame-time distribution while scrolling the uncached corpus, time
  to fill after a jump, total generation time, peak and steady memory.

## Phase 3: full-screen view

- Thumbnail as the placeholder straight away; ±2 neighbours preloaded,
  cancelled when no longer needed, at most two decodes at once.
- **Screen-size decode by default:** JPEGs via `turbojpeg` at the smallest
  scaling factor that still covers the screen (a 34 MP photo at 1/2 or 1/4 on
  a 1440p/4K monitor), other formats decoded and scaled with
  `fast_image_resize` on the worker. Several times faster to decode (helps
  holding ←/→) and ~8–30 MB to upload instead of ~136 MB.
- Compare with glycin (out of process, sandboxed, memfd, but always full size
  on Linux; glycin 4.0's in-process "builtin" mode is for other platforms).
- Measure: Enter to placeholder and to sharp image, stalls while switching,
  holding → at 30/s (dropped frames, time to the final image after release).

## Phase 3b (only if needed): full-resolution zoom

Tiles of ~1024² at full resolution, created only for the visible region and a
few per frame, so zooming never uploads a whole 34 MP image in one frame.

## Maybe: uploading off the main thread

Only if screen-size textures and tiles aren't enough. Both fragile:
`GdkGLTextureBuilder` with a shared GL context (GTK defaults to Vulkan on
Wayland, which would have to import or copy it), or dmabufs filled on a worker
(`GdkDmabufTextureBuilder`; NVIDIA support has been poor).

## Phase 4: compare and decide

`bench/run.sh` on both apps, results in `bench/RESULTS.md`, then a try on rei
for feel. Decide: continue the port, adjust and re-measure, or drop it.

## Working rules

- Numbers reported after each phase before the next.
- Commits on `rust-spike` only; CI runs only on `master` and pull requests.
