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
- **2b result:** per image, JPEG 10–45 ms (4–24 MP; the entropy decode is the
  floor), PNG 50 → 26 ms (`png` + SIMD resize), WebP unchanged at ~68 ms
  (GdkPixbuf's loader already uses libwebp's scaling; libwebp's decode is the
  floor). First screen ~30–400 ms cold (varies with startup), the corpus in
  ~12.4 s (bound by the 5 background workers, not decoding). EXIF rotation now
  fits the rotated size (2a gave portraits 320×427). Memory: glibc arenas kept
  the workers' decode buffers (~230 MB after a cold run); a fixed 1 MB mmap
  threshold and 4 arenas (`mallopt`) cut the cold peak ~844 → ~740 MB. Warm
  with GL: 479 MB peak (138 MB of it the 300 cached textures, ~120 MB mapped
  libraries), no dropped frames.
- **2c:** a variant using the freedesktop thumbnail cache
  (`~/.cache/thumbnails/x-large`, 512 px, MD5 of the file URI, PNG with
  `Thumb::URI`/`Thumb::MTime`/`Thumb::Size`), shared with file managers:
  folders already browsed in one show instantly. Costs PNG encode/decode
  (on workers). A decision for later, since it changes the app's cache.
- **2a result:** first screen in ~0.4 s (TS: ~9.8 s cold), the whole corpus
  generated in ~15–17 s in the background; scrolling p50/p95 16.6/17.0 ms
  (TS: 51/146) with 3 dropped frames (TS: 335). Workers run at nice 10.
  The remaining stalls (~250 of ~12 ms, one per row of new tiles, and
  ~200 ms on a jump) are GTK's Vulkan renderer on NVIDIA's driver, a few ms
  per new texture; with `GSK_RENDERER=ngl` they drop to ~20 of ≤14 ms and a
  ~50 ms jump. The TypeScript app gains from GL too (holding →: 11 → 4
  dropped frames warm). Memory peaks at ~770–910 MB, still to explain.
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

- **Result:** decoding at the window's size, on 2 workers, with the shown
  image first and the queue replaced on every move. Warm with GL: the
  placeholder at once, the sharp image in ~35 ms (TS: ~130 ms), holding → with
  no dropped frames (TS: 4; 11 with Vulkan) and the last image sharp on
  release. Decoding at full resolution instead (`VITRINE_VIEWER=full`) keeps
  opening fast but makes holding → drop 17–18 frames with stalls up to ~300
  ms (uploads), and peaks ~270 MB higher: screen-size decoding is the default.
  Glycin wasn't built: it always decodes at full size, and its JPEG loader
  (zune-jpeg) can't decode smaller, so it does strictly more work than the
  full-resolution variant measured here.
- Found on the way: GTK's `GridView` keeps ~390 tiles bound (rows around the
  viewport); with a 300-texture cap, off-screen tiles evicted visible ones
  (the view opened without its placeholder). Bound tiles' textures are now
  never evicted, plus up to 100 for tiles scrolled away.

- **Preloading the grid's selection:** the image selected in the grid is
  decoded in the background (each change replaces the queue), and closing the
  view keeps it; select, wait 300 ms, open: sharp at once (was ~20–56 ms).
  Dropping the previous preload freed its buffer (33 MB at 4K) on the main
  thread, in the selection handler (0.7 ms median, 4 ms worst: the highlight
  lagged a frame at 144 Hz); buffers over 1 MB are now freed on a janitor
  thread (8 µs). Still: the frame that first shows a 4K image uploads its
  texture (~55 ms), preloaded or not.
- **Opening at the top:** batches arrive in any order and are sorted as they
  come, so the automatic selection stayed on whichever image arrived first,
  and the grid kept it in view: a 7,756-image folder opened halfway down
  (image 3,664 selected). While loading (scan and sorting), the first image
  is now kept selected and the grid at the top, until a press, key or scroll
  in the grid.
- **Selection style:** the default theme's animated highlight looked choppy
  on a mouse click, next to 144 Hz scrolling (in the headless session GTK drew
  such short animations at ~60 fps). For now an outline fades in over 120 ms;
  revisit in the styling pass.

## Phase 3b: full-resolution zoom

Done in port batch 2 (below): 512² tiles, only the visible ones drawn, at
most 3 new ones per frame.

## Port batch 2: the full-screen view

As the TypeScript app (ZoomableImage.ts, AutoHide.ts, the view's part of
Window.tsx), instant zoom steps, no pinch:

- `zoomable.rs`: a widget drawing the image with its transform. Scroll zooms
  around the cursor (1.2× steps, fit to 8× actual size); +/-/0; double-click
  toggles fit/100% (2× fit for small images); drag pans, clamped; at fit, the
  left/right sixth moves to the previous/next image (arrow cursors, a hand
  when zoomed); s sharp pixels; [ ] rotate, h v flip (view only, reset per
  image).
- Resolution levels: a texture of the view's size in device pixels for the
  whole image, drawn at exactly its own pixels at fit (decoded again when
  the view's size changes); zooming past it decodes the full
  resolution on a worker (`preview.rs`, ahead of preloads) into 512² tiles
  with a 1-pixel overlap (no seams under smoothing). Only visible tiles are
  drawn, so uploaded, at most 3 new ones per frame (~3 MB); the rest show the
  lower resolution for a frame or two.
- Sharpness, softer than the TypeScript app at first, for two reasons:
  - Decoding for the monitor and drawing it scaled to the window (×0.9–0.97,
    bilinear). Now the texture fits the view and is drawn 1:1, resized with
    Lanczos3 (was Catmull-Rom): at scale 1, the view at fit went from 67% to
    99% of the edge contrast of ImageMagick's Lanczos downscale.
  - At display scales above 1 (the author's 1.5×), a scaled texture node with
    the linear filter came out blurred even when drawn 1:1: the compositor's
    output had 58% of the texture's own edge contrast, with every renderer.
    A plain texture node (GTK's default filter, as the TypeScript app uses)
    keeps all of it; the view at 1.5× now has 95% of the ideal's (the rest is
    libjpeg-turbo's 3/8 reduced decode; 1/2 would give 97%).
  Measured with `VITRINE_PROBE_GRIM` (device-pixel captures) on a synthetic
  test image; the in-app render is at scale 1 and can't show the second one.
- GIFs: frames decoded on a thread of their own, 4 ahead, fitted to the
  monitor; swapped in on the frame clock. (The TypeScript app decoded each
  frame on the main thread.)
- `view.rs`: the page, with the fullscreen and close buttons and the file's
  name and resolution over the image; f / the button toggles fullscreen,
  leaving the view restores the window if the view made it fullscreen;
  `autohide.rs` fades the controls and hides the cursor after 2 s without
  mouse movement in fullscreen. The TypeScript app's icons are packaged.
  Also: q closes the view, e opens the selection from the grid, and keys with
  Ctrl/Alt/Super held are left alone.
- NVIDIA: GL is picked as on `master` (`prefer_gl_on_nvidia`): with Vulkan,
  panning at 100% drew 76 ms frames (each tile panned into view is a new
  texture).
- **Result** (4K, 144 Hz, 1.5×, warm, GL; `bench/results/rs-warm-4k-gl.txt`):
  zoom to 100% on a 48 MP JPEG, full detail in ~530 ms, 3 late frames; pan at
  100%, no late frames (p95 7.4 ms); GIF with no stalls; fullscreen and
  auto-hide checked by the probe. Not testable headless (no pointer):
  double-click, edge clicks, dragging, cursors, the controls coming back on
  movement. Peak memory rises by the zoomed image's tiles (a 48 MP image is
  ~190 MB of tiles, twice that briefly while splitting).

## Maybe: uploading off the main thread

Only if screen-size textures and tiles aren't enough. Both fragile:
`GdkGLTextureBuilder` with a shared GL context (GTK defaults to Vulkan on
Wayland, which would have to import or copy it), or dmabufs filled on a worker
(`GdkDmabufTextureBuilder`; NVIDIA support has been poor).

## Phase 4: compare and decide

`bench/run.sh` on both apps, results in `bench/RESULTS.md`, then a try on rei
for feel. Decide: continue the port, adjust and re-measure, or drop it.

- **Results written** (`bench/RESULTS.md`): the numbers favour continuing the
  port. Still to do: the try on rei.

## Working rules

- Numbers reported after each phase before the next.
- Commits on `rust-spike` only; CI runs only on `master` and pull requests.
