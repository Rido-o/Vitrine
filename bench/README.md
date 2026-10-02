# Benchmark

Measures Vitrine on a generated corpus in a headless sway (1920×1080 at
60 Hz unless set otherwise), with its own, initially empty, thumbnail cache
and state. It was built to compare the TypeScript app with the Rust port;
`RESULTS.md` has that comparison, and `results/ts-*` the TypeScript app's raw
runs.

```sh
bench/make-corpus.sh ~/Pictures/Wallpapers   # once: 3,000 images, ~4 GB
                                             # (JOBS=4 at a time, ~1.2 GB each)
bench/run.sh               # cold cache
bench/run.sh warm          # again, with the thumbnails already on disk
BENCH_DISPLAY=$WAYLAND_DISPLAY bench/run.sh   # watch it in a real window
BENCH_OUTPUT=3840x2160@144Hz BENCH_SCALE=1.5 bench/run.sh warm
```

`VITRINE_PROBE_SHOTS=DIR` saves the window as drawn at a few points
(zoom-fit, zoom-100, pan, rotated-flipped) to DIR/NAME.png, to check what the
view shows, with the view's texture as NAME-texture.png. That render is at
scale 1; with `VITRINE_PROBE_GRIM=path/to/grim` the compositor's output is
saved too, in device pixels (NAME-device.png), to compare with the texture at
display scales above 1 (and to capture popovers, which the render leaves
out). `BENCH_SWAY_EXTRA` adds lines to the headless sway's config (e.g.
`default_border none`, so the window is at the output's corner).

The corpus is opened with subfolders (`-r`); `BENCH_ARGS` replaces the
arguments before the folder, which can also be a file.
`BENCH_PROBE=ui BENCH_ARGS= bench/run.sh warm DIR` runs the UI check instead
of the benchmark: sorting, subfolders, the ⋯ menu, copying, the shortcuts
window, the wallpaper command (with `VITRINE_WALLPAPER_COMMAND`), the folder
entry and its history, properties (and EXIF orientation outside JPEG, with
`props/rotated.png` and `props/rotated.webp`) and the empty state on a small
folder (rescan and watching too, changing files, only if DIR holds a
`.vitrine-probe-scratch` file), printed as `ui` lines, with screenshots under
`VITRINE_PROBE_SHOTS`. Its trash and undo checks trash files for real, so they
only run with `VITRINE_PROBE_TRASH=1`: use it only on the scratch folder, with
a data folder of its own on the same filesystem, `XDG_DATA_HOME=<scratch>/data`
(on another filesystem, trashed files go to that filesystem's own
`.Trash-<uid>`).

`BENCH_REAL_CACHE=1` runs with the app's real cache and state instead of the
benchmark's (e.g. `bench/run.sh warm ~/Pictures` on a folder whose
thumbnails exist).

`BENCH_OUTPUT` and `BENCH_SCALE` set the headless display (default
1920×1080 at 60 Hz, scale 1). Match the real monitor where it matters: at
60 Hz a frame has 16.7 ms, so 8–15 ms stalls don't show as late frames that
are obvious at 144 Hz (6.9 ms).

Raw runs are kept in `results/` (`rs-<mode>.txt` for the Rust app, `-ngl` or
`-gl` for GTK's GL renderer, `-4k` for 3840×2160 at 144 Hz, scale 1.5;
overwritten by the next run of the same kind; the header line has the
display, date and commit).

Run both renderers: on NVIDIA's driver, GTK's default Vulkan renderer costs
several ms of main-thread time per new texture (~12 ms per row of new
thumbnails, ~100 ms `ioctl`s when many arrive at once), which the GL renderer
(`GSK_RENDERER=gl`, the app's default there) doesn't. `BENCH_WRAP` prefixes
the app's command, e.g.
`BENCH_WRAP="perf record -k CLOCK_MONOTONIC --call-graph dwarf -o x.perf"`;
the lines end with `t0_us` (monotonic) to find a scenario in a profile.

The probe is built into the app (`src/probe/`, enabled with
`VITRINE_PROBE=1`, or `ui`).

## Corpus

`make-corpus.sh` derives every image from the source folder's images
(cropped, resized and hue-shifted so each file differs), in folders
`a0`–`a9`/`b0`–`b4` with every 7th image at the top:

| kind | count | size | format |
|---|---|---|---|
| huge | 50 | 8000×6000 (48 MP) | JPEG |
| large | 250 | 6000×4000 (24 MP) | JPEG |
| medium | 700 | 4000×3000 (12 MP) | JPEG |
| small | 1300 | 2560×1600 (4 MP) | JPEG |
| webp | 250 | 3840×2160 | WebP |
| alpha | 200 | 1920×1080, transparent corners | PNG |
| portrait | 250 | 4000×3000 stored, EXIF-rotated | JPEG |
| gif | 10 | 960×540, 12 frames 80 ms apart | GIF |

## Scenarios

In order, in one run:

| scenario | what | extra fields |
|---|---|---|
| `scan` | from the window appearing until the item count has been stable for 1 s | `first_ms` (first item), `done_ms` (last change), `items` |
| `scan_detail` | from starting the walk, as the app sees it | `first_us`, `done_us`, `batches`, `insert_max_us` (longest main-thread insert) |
| `background` | background generation finished | `generated`, `done_ms` (from starting the walk) |
| `fill_first` | the first screen's thumbnails | `fill_ms`, `top_px` and `selected` (where the grid is once loaded; 0 and 0 expected) |
| `scroll` | top to bottom at 4,000 px/s, then until every bound tile has a thumbnail | `scroll_ms`, `fill_after_ms` |
| `jump` | to the middle in one step, until filled | `fill_ms` |
| `sort` | click Date, Size, Random, Name, 300 ms apart | `date_us` etc. (each click on the main thread), `kept` (clicks after which the same image was still selected; 4 expected) |
| `open` | activate the first image (from the top), 1.5 s | `placeholder_ms`, `sharp_ms` |
| `hold` | → 60 times at 30/s (the view moves on at most 15 times a second), then until the last image is sharp | `moves`, `last_sharp_ms` |
| `close` | Esc, 1 s | |
| `open_selected` | select the 11th image, wait 300 ms, open it | `dwell_ms`, `sharp_ms` |
| `zoom` | open the first `huge-` image and zoom to 100% at the centre, 1.5 s | `detail_ms` (until every visible full-resolution tile is drawn) |
| `pan` | then pan across it for 1.5 s | `pan_ms`, `detail_after_ms` |
| `gif` | play the first GIF for 3 s | `frames_shown` |
| `fullscreen` | f in the view, wait 2.5 s, Esc | `entered_ms`, `controls` and `hidden` (both 2: the controls auto-hid), `left_ms` |
| `hold_key` | hold → for real for 3 s (a virtual keyboard, `wtype`; GTK repeats the key), then watch 6 s | `shows_held`, `shows_after`, `last_show_after_ms`, `settle_ms` (first 100 ms after release using under 10 ms of CPU), `cpu_held`/`cpu_after` (ms per thread: `main`, `preview`, `thumbnail`, `janitor`), `system_held`/`system_after` (ms, all cores) |
| `idle` | 2 s doing nothing | |
| `memory` | at the end | `rss_mb`, `hwm_mb` (peak) |

Each timed scenario also reports:

- `frames`, `p50`/`p95`/`p99`/`max`: frame-clock intervals in ms (16.7 at
  60 Hz). A tick callback keeps the clock running, so a blocked main loop shows
  as a long interval.
- `over25`: intervals over 25 ms (visibly dropped frames at 60 Hz).
- `late`: intervals over 1.5 refresh intervals of the headless display
  (10.4 ms at 144 Hz). Only meaningful while frames are drawn continuously
  (`scroll`); when nothing changes GTK draws less often, so idle scenarios
  count most frames as late.
- `stalls`, `stall_sum`, `stall_max`: main-loop iterations over 8 ms, from a
  2 ms high-priority timer.

`-1` means a wait timed out. Absolute numbers depend on the machine and the
headless compositor; compare runs from the same session.
