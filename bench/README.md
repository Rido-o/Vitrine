# Benchmark

Compares the TypeScript app with the gtk4-rs spike (`rust/`) on the same
corpus and scenarios. Each app runs in a headless sway (1920×1080 at 60 Hz)
with its own, initially empty, thumbnail cache.

```sh
bench/make-corpus.sh ~/Pictures/Wallpapers   # once: 3,000 images, ~4 GB
                                             # (JOBS=4 at a time, ~1.2 GB each)
bench/run.sh ts            # the TypeScript app, cold cache
bench/run.sh rs            # the spike, cold cache
bench/run.sh rs warm       # again, with the thumbnails already on disk
BENCH_DISPLAY=$WAYLAND_DISPLAY bench/run.sh rs   # watch it in a real window
```

Raw runs are kept in `results/` (`<app>-<mode>.txt`, overwritten by the next
run of the same kind; the header line has the date and commit).

`bench-ts` is the TypeScript app bundled with `probe-ts.tsx` as its entry
(`nix build .#bench-ts`); the spike has the same probe built in
(`rust/src/probe.rs`, enabled with `VITRINE_PROBE=1`). Both print the same
`RESULT` lines.

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

## Scenarios

In order, in one run:

| scenario | what | extra fields |
|---|---|---|
| `scan` | from the window appearing until the item count has been stable for 1 s | `first_ms` (first item), `done_ms` (last change), `items` |
| `scan_detail` | spike only: from starting the walk, as the app sees it | `first_us`, `done_us`, `batches`, `insert_max_us` (longest main-thread insert) |
| `fill_first` | the first screen's thumbnails | `fill_ms` |
| `scroll` | top to bottom at 4,000 px/s, then until every bound tile has a thumbnail | `scroll_ms`, `fill_after_ms` |
| `jump` | to the middle in one step, until filled | `fill_ms` |
| `open` | activate the first image (from the top), 1.5 s | `placeholder_ms`, `sharp_ms` |
| `hold` | → 60 times at 30/s, then until the last image is sharp | `last_sharp_ms` |
| `close` | Esc, 1 s | |
| `idle` | 2 s doing nothing | |
| `memory` | at the end | `rss_mb`, `hwm_mb` (peak) |

Each timed scenario also reports:

- `frames`, `p50`/`p95`/`p99`/`max`: frame-clock intervals in ms (16.7 at
  60 Hz). A tick callback keeps the clock running, so a blocked main loop shows
  as a long interval.
- `over25`: intervals over 25 ms (visibly dropped frames).
- `stalls`, `stall_sum`, `stall_max`: main-loop iterations over 8 ms, from a
  2 ms high-priority timer.

`-1` means a wait timed out. Absolute numbers depend on the machine and the
headless compositor; compare the two apps from the same session.
