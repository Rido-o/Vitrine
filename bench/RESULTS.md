# Spike results

(A record of the comparison that decided the port: the spike then lived in
`rust/`, with its plan in `rust/PLAN.md` (see the git history); it is now the
app, and the TypeScript app is gone.)

The gtk4-rs spike (`rust/`, see `rust/PLAN.md`) against the TypeScript app,
on the same corpus and scenarios (`bench/README.md`). Raw runs are in
`results/`; the numbers below are from the runs at `ab9d0e4` (TypeScript
app, with the Phase 3 probe; its warm runs at `2909b5e`, see "Correction")
and `5844d4b` (spike, end of Phase 3).

## Verdict

Once every thumbnail is on disk, both apps scroll about equally smoothly
(56–60 frames per second). The spike is much better everywhere else:

- **While thumbnails are being generated** (a new folder, or one the
  TypeScript app hasn't finished; it doesn't finish this corpus within a
  run), the spike draws ~3.5× as many frames per second while scrolling and
  fills the first screen ~25× sooner.
- **Opening an image** shows the thumbnail at once and the sharp image 3–4×
  sooner.
- **Holding ←/→** drops 0–4 frames against 4–11.
- **Memory peaks** 16–61% lower in three of the four configurations. The
  exception is cold on Vulkan, where both peak at about 870–880 MB.

With the GL renderer (`GSK_RENDERER=ngl`, now `gl`) the spike drops at most
a handful of frames in any scenario.

**Recommendation: continue the port.** The spike does far less than the app
(see "What the spike doesn't do"), so some of the gap may close as features
come over. The gains come from structure, not from missing features:
decoding on worker threads, memory freed as soon as it's dropped, and decoding
only the pixels shown. None of those depends on what gets ported.

## Setup

- AMD Ryzen 5 5600X (6 cores, 12 threads), 15 GB, NVIDIA RTX 3070 Ti with
  driver 595.71.05, GTK 4.22.4, gjs 1.88.1, Rust 1.98.1.
- A headless sway at 1920×1080, 60 Hz; the window is 1600×1000 (scale 1).
- 3,000 images in 51 nested folders (`make-corpus.sh`): JPEGs from 4 to
  48 MP (some EXIF-rotated), 8 MP WebP and 2 MP transparent PNGs, 4.1 GB.
- **Cold:** empty thumbnail cache. **Warm:** every thumbnail already on disk.
  Files are in the page cache in both.
- Frame intervals are 16.7 ms at 60 Hz. "Dropped" counts intervals over
  25 ms; a stall is a main-loop iteration over 8 ms.

## Headline numbers

Vulkan is what users get by default; GL is the fix on NVIDIA (see "Findings").

| | TS, Vulkan | spike, Vulkan | TS, GL | spike, GL |
|---|---|---|---|---|
| First screen of thumbnails (cold) | 9.8 s | 0–0.4 s | 9.9 s | 0–0.4 s |
| Frames per second scrolling, cold | 16 | 57 | 16 | 58 |
| Frames per second scrolling, warm | 56 | 57 | 60 | 58 |
| Open an image: sharp (warm) | 128 ms | 40 ms | 138 ms | 35 ms |
| Hold → 60 times: dropped frames (warm) | 11 | 4 | 4 | 0 |
| Peak memory (warm) | 1,340 MB | 732 MB | 1,640 MB | 638 MB |

The spike's first screen varies between runs (it competes with start-up and
every worker starting at once).

## Scanning and start-up

| | TS cold | TS warm | spike cold | spike warm |
|---|---|---|---|---|
| All 3,000 items listed, from the window | 1.2 s | 1.6 s | at once | at once |
| Worst stall while scanning | 264 ms | 710 ms | 226 ms | 163 ms |

The spike's walk takes ~7 ms on a worker thread; it is done before the window
has drawn. Both apps' worst stall here is start-up (GTK loading its icon theme
and the first frame, ~170–250 ms), not scanning. The spike's slowest insert
is ~8.5 ms, when the grid creates its first screen of tiles; later batches of
512 insert in ~80 µs.

## Thumbnails and scrolling

Scrolling top to bottom at 4,000 px/s (about 20 s), Vulkan / GL:

| | TS cold | spike cold | TS warm | spike warm |
|---|---|---|---|---|
| Frames drawn | 437 / 445 | 1,168 / 1,176 | 1,150 / 1,220 | 1,169 / 1,172 |
| Median frame | 52 / 50 ms | 17.0 / 17.3 ms | 16.5 / 16.7 ms | 17.0 / 17.2 ms |
| 95th percentile frame | 150 / 147 ms | 19.5 / 18.9 ms | 18.6 / 17.9 ms | 18.7 / 18.5 ms |
| Dropped frames | 336 / 344 | 41 / 5 | 33 / 13 | 30 / 10 |
| Main thread blocked in total | 10.2 / 9.1 s | 3.3 / 0.6 s | 3.0 / 0.3 s | 2.9 / 0.4 s |
| Scroll took (target ~20.4 s) | 27.0 / 27.0 s | 20.4 s | 20.4 s | 20.4 s |
| Jump to the middle, until filled | 8.0 / 7.7 s | 0.35 s / 46 ms | 223 / 218 ms | 0.34 s / 54 ms |

Generating all 3,000 thumbnails in the background takes the spike 11–14 s;
the TypeScript app hadn't finished by the end of a run. Per image on a
worker: 4 MP JPEG ~10 ms, 12 MP ~22 ms, 24 MP ~40 ms, 48 MP ~75 ms, 8 MP WebP
~68 ms, 2 MP PNG ~26 ms.

## Full-screen view

Opening the first image from the grid, then holding → for 60 presses at 30
per second, then Esc. Warm, Vulkan / GL:

| | TS | spike |
|---|---|---|
| Placeholder (thumbnail) shown | 126 / 135 ms | 0 / 0 ms |
| Sharp image shown | 128 / 138 ms | 40 / 35 ms |
| Open: dropped frames | 2 / 2 | 1 / 0 |
| Hold →: dropped frames | 11 / 4 | 4 / 0 |
| Hold →: worst stall | 44 / 18 ms | 12 ms / none |
| Close: dropped frames | 1 / 1 | 4 / 0 |

Cold runs are similar for the spike (sharp in 30–56 ms, no dropped frames
holding → on Vulkan). For the TypeScript app, cold is 183–211 ms to sharp and
32–39 dropped frames holding →.

The spike decodes at the window's size. Decoding at full resolution instead
(`VITRINE_VIEWER=full`, a switch since removed; warm, GL) opens as fast (29 ms), but holding → drops
17 frames with stalls up to 294 ms, and memory peaks at 908 MB instead of
638 MB: uploading full-size textures on the main thread is what costs.

## Memory

Peak resident memory over a whole run:

| | TS, Vulkan | spike, Vulkan | TS, GL | spike, GL |
|---|---|---|---|---|
| Cold | 867 MB | 883 MB | 950 MB | 801 MB |
| Warm | 1,340 MB | 732 MB | 1,640 MB | 638 MB |

The TypeScript app's peaks vary a lot between identical runs (1,063 and
1,340 MB warm on Vulkan, minutes apart), with its garbage collector's timing;
the spike's vary by a few tens of MB.

At the end of a warm GL run the spike holds 592 MB. Of that, ~120 MB is mapped
files (libraries and fonts, mostly shared with other processes) and ~30 MB GPU
driver mappings. The thumbnails are the ~390 textures of bound tiles plus up to 100
for tiles scrolled away, about 220 MB. Cold runs peak higher while the
workers decode: capping glibc's arenas and fixing its mmap threshold
(`mallopt` in `main.rs`) took the cold peak from ~844 to ~740–800 MB.

## Findings

- **GTK's Vulkan renderer on NVIDIA costs several ms of main-thread time per
  new texture.** That's ~12 ms per row of new thumbnails, and 100–300 ms when
  a screenful arrives at once (e.g. after a jump). It shows up as NVIDIA
  driver `ioctl`s. The GL renderer (`GSK_RENDERER=ngl`) doesn't have it. It's
  a known driver problem, not ours. **The TypeScript app gains from GL too:**
  warm, scrolling drops 13 frames instead of 33 and holding → 4 instead of
  11. `master` now picks GL itself when NVIDIA's driver is loaded.
- **GdkPixbuf's async API decodes on the main thread** (it only reads in the
  background). This was the TypeScript app's biggest cost; in the spike every
  decode runs on a worker.
- **`GridView` keeps ~390 tiles bound**, far more than are visible. A
  300-texture cache lets off-screen tiles evict visible ones. The spike never
  evicts a bound tile's texture. The TypeScript app had the same thrash (its
  view opened without the placeholder for ~130 ms); `master` now keeps bound
  tiles' textures too.
- **Decode only the pixels shown.** Thumbnails decode JPEGs at a reduced size
  (libjpeg-turbo's 1/2–1/8 scaling), and the full-screen view decodes at the
  window's size. Both cut decode time, memory, and above all upload time.
- **Hand GTK premultiplied RGBA.** Anything else (e.g. plain RGB) GTK converts
  on the main thread when first drawn.
- **Glycin wasn't tried in the spike.** It always decodes at full size, and
  its JPEG loader can't decode smaller, so it does more work than the
  full-resolution variant measured above.

## At 4K, 144 Hz

The runs above use a 1920×1080, 60 Hz display, where a frame has 16.7 ms. On
the author's monitor (3840×2160 at 144 Hz, scale 1.5), a frame has 6.9 ms,
and the TypeScript app's short stalls turn into visible hitches. Warm, GL,
`master` at `fd26708` (both fixes) against the spike:

| | TS | spike |
|---|---|---|
| Frames drawn scrolling | 2,210 | 1,986 |
| Median / 95th percentile frame | 6.7 / 7.8 ms | 6.8 / 6.9 ms |
| Late frames (over 10.4 ms) | 63 | 3 |
| Worst frame | 135 ms | 42 ms |
| Main thread blocked in total | 185 ms | 67 ms |
| Open an image: sharp | 143 ms | 20 ms |
| Peak memory | 1,803 MB | 978 MB |

(At the same speed the two scroll for different times, 15.2 s against
13.4 s, because their scroll distances differ: the windows lay out
differently, e.g. the TypeScript app has a toolbar and an info bar.) This matches how the two feel: the
spike scrolls noticeably smoother on that monitor, even with every thumbnail
cached.

## Correction

The first version of this document compared against "warm" runs of the
TypeScript app that weren't warm. Each followed a cold run, and the app
hadn't finished generating thumbnails in the background by the end of it, so
the "warm" run was still generating, on the main thread. Those runs gave the
TypeScript app 16–41 frames per second scrolling warm, 156–455 ms to open an
image and 8–40 dropped frames holding →. The warm numbers above are from runs
with every thumbnail on disk. The spike finishes generating within its cold
run, so its warm runs were warm. The cold comparisons are unaffected.

## What the spike doesn't do

Only a recursive scan sorted by path, the grid with thumbnails, and a
full-screen view with ←/→ and Esc. Missing, compared with the TypeScript app:
sorting, the directory entry and history, file watching, trash and undo,
properties, wallpaper, menus and shortcuts, zoom/pan/rotate/flip, GIF
playback, the info bar, styling, fractional scaling and re-decoding on
resize, the thumbnail cache migration, and the Home Manager module. Some of
these add main-thread work (labels, the info bar, watching). None changes how
decoding and texture handling work, which is where the gap comes from.

## Reproduce

```sh
bench/make-corpus.sh ~/Pictures/Wallpapers
bench/run.sh ts cold; bench/run.sh ts warm
bench/run.sh rs cold; bench/run.sh rs warm
GSK_RENDERER=ngl bench/run.sh rs warm   # and so on for GL
```
