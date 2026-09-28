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
  To do: keep the automatic selection on the first image while a scan runs
  (it follows the first image found as sorting moves it).

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

- **Results written** (`bench/RESULTS.md`): the numbers favour continuing the
  port. Still to do: the try on rei.

## Working rules

- Numbers reported after each phase before the next.
- Commits on `rust-spike` only; CI runs only on `master` and pull requests.
