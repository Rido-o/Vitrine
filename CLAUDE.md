# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Vitrine is a GTK4 image viewer and wallpaper picker in Rust with gtk4-rs, packaged as a Nix flake. The README is the main documentation (features, keys, layout, build, gotchas, roadmap); read it before non-trivial changes.

## Commands

```sh
nix build                          # build the package
nix run . -- DIR                   # build and run (add -r for subfolders)
VITRINE_WALLPAPER_COMMAND=set-wallpaper nix run . -- DIR
nix fmt                            # alejandra (Nix files)
nix flake check                    # the build, clippy (-D warnings), rustfmt
nix develop                        # cargo, clippy, rustfmt, rust-analyzer, dart-sass
nix develop -c cargo build --release   # quicker rebuilds while iterating
```

New files must be `git add`ed before the flake can see them. CI (`.github/workflows/check.yml`) runs `nix flake check` and `nix build` on pushes to `master` and pull requests; after pushing, check the run passed.

### Verifying changes

There are no unit tests, so:

- `nix flake check` builds the crate, runs clippy with warnings as errors, and checks formatting (`cargo fmt` in the dev shell). Keep it passing; don't silence a lint with `#[allow]` without a comment saying why.
- `nix build`, then `$(nix build --print-out-paths)/bin/vitrine --help`: catches wrapper and startup problems without opening a window.
- The app has a built-in probe (`src/probe/`), run in a headless sway by `bench/run.sh` (see `bench/README.md`):
  - `VITRINE_PROBE=1` (the default in `bench/run.sh`) is the benchmark: frame times, stalls, decode times, memory. Measure at the author's display where it matters: `BENCH_OUTPUT=3840x2160@144Hz BENCH_SCALE=1.5 GSK_RENDERER=gl bench/run.sh warm`.
  - `BENCH_PROBE=ui BENCH_ARGS= bench/run.sh warm DIR` drives the controls (sorting, menus, copy, history, properties, rescans, watching) and prints what it sees; extend it for new behaviour. Use a folder of synthetic images, not the user's: files are added, changed and removed when it holds `.vitrine-probe-scratch`. Its trash checks only run with `VITRINE_PROBE_TRASH=1`, on a private bus with its own `XDG_DATA_HOME`, never against the real trash.
  - `VITRINE_PROBE_SHOTS=DIR` (with `VITRINE_PROBE_GRIM` for device pixels and popovers) saves screenshots to look at.
- Anything that needs a real pointer or the user's eye (dragging, hover, how it looks at 1.5×) needs the user to run it; say what to check.

## Code

- `src/main.rs` owns the `gtk::Application`, command line, CSS and startup tuning; `src/window/` owns layout and interaction (`Window` in `mod.rs`, one file per part: toolbar, grid, info bar, navigation to and from the view, menu, delete, keys, toast); `library.rs` (scanning, sorting, the list model, rescans, watching), `thumbnails/` (worker pool, disk cache, the grid's textures), `view/` (the full-screen view: page, decodes, zoomable widget, autohide), `decode/` (with colour profiles), `desktop/` (file manager, wallpaper command, trash), `properties.rs`, `shortcuts.rs`, `actions.rs` and `history.rs` are the pieces. The README's "Layout" lists them all.
- All GTK work stays on the main thread; worker threads only produce pixels (see the README's "Layout" and "Gotchas"). Keep decoding, file reading and big frees off the main thread.
- Rust style: `cargo fmt` (rustfmt defaults), edition 2024; Nix: alejandra. Styles are `style/style.scss` (with `style/theme.scss`), compiled into the binary by `build.rs`.
- Keep comments sparse: explain why, not what.
- Known traps (details in the README's "Gotchas"): the GL renderer on NVIDIA; draw the view with a plain texture node, decoded at the view's device-pixel size; never evict bound tiles' thumbnails; the package builds its own GdkPixbuf `loaders.cache` and wraps by hand (`dontWrapGApps`) so WebP works; while a folder loads, the first image stays selected; a window's state is an `Rc` its handlers hold weakly, kept alive by its `destroy` handler.
- The app ID is `io.github.Rido_o.Vitrine`; data lives in `~/.cache/vitrine/thumbnails-4` and `~/.local/state/vitrine/history`. Changing names or paths needs a migration (outdated thumbnail caches are deleted by `thumbnails::housekeeping`, since thumbnails regenerate; the history is moved, as `history::migrate` does).
- Nothing may assume the author's setup: machine-specific values (wallpaper command, folders) come from the command line, `VITRINE_WALLPAPER_COMMAND`, or Home Manager options (`hm-module.nix`, `programs.vitrine.*`).

## Docs

- Update the README in the same change when behaviour, keys, options, layout or build change. The roadmap lives in the README's "Roadmap" section; remove items when done.
- The licence is undecided; don't add one.

## Relationship to the author's NixOS config

The author's config (`~/.nix`, `Rido-o/nix`) consumes this repo as the `vitrine` flake input (following its nixpkgs) and imports `homeManagerModules.default` on the desktop host `rei`. Changes here reach it only after a push and `nix flake update vitrine` there; to try unpushed changes on rei: `nh os switch -- --override-input vitrine path:$HOME/Projects/Vitrine`. Don't edit `~/.nix` from this repo's sessions unless asked.

## Workflow

- For anything beyond a trivial edit, show a detailed plan first and wait for approval before changing files.
- After changes: verify (build plus the checks above), summarise, then ask before committing. Once approved, commit and push to `master`.
- Offer one commit per scope; use a single commit if asked.
- Commit messages: a short imperative sentence, capitalised, no prefix or trailing full stop, e.g. `Add a standalone flake`, `Prune stale thumbnails on startup`.
- When a fix isn't obvious, find the cause (headless tests, logs, upstream docs) before trying changes; avoid timer or retry workarounds.
