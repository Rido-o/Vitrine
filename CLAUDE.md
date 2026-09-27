# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Vitrine is a GTK4 image viewer and wallpaper picker in TypeScript for GJS, with gnim for JSX, packaged as a Nix flake. The README is the main documentation (features, keys, layout, build, gotchas, roadmap); read it before non-trivial changes.

## Commands

```sh
nix build                          # build the package
nix run . -- DIR                   # build and run (add -r for subfolders)
VITRINE_WALLPAPER_COMMAND=set-wallpaper nix run . -- DIR
nix fmt                            # alejandra (Nix files)
nix flake check                    # includes checks.typecheck (tsc --noEmit)
nix develop                        # gjs, esbuild, dart-sass, tsc; links node_modules
```

New files must be `git add`ed before the flake can see them.

### Verifying changes

There are no tests, so:

- `nix flake check` type-checks `src/` with `tsc` (esbuild alone strips types). Keep it passing; don't silence errors with `any` or `@ts-ignore` without a comment saying why. The `@girs` packages are deliberately pinned to the `4.0.0-rc.17` generation (see "Build" in the README).
- `nix build`, then `$(nix build --print-out-paths)/bin/vitrine --help`: GApplication prints help only after the whole bundle has loaded, so this catches import and load-time errors without opening a window.
- Logic that doesn't need a window (scanning, sorting, thumbnails) can be tested headless: bundle a small entry that imports the module with esbuild and run it with the package's own `gjs` and `GI_TYPELIB_PATH` (both from the wrapper, `bin/.vitrine-wrapped` and `strings bin/vitrine`); a different gjs mismatches the typelibs. See "Gotchas" in the README.
- Anything visual needs the user to run it; say what to check.

## Code

- `src/main.tsx` owns the `Gtk.Application` and command line; `src/Window.tsx` owns layout and interaction; `Library.ts` (async scanning, sorting, the list model), `Thumbnails.ts`, `History.ts` and `ZoomableImage.ts` are the pieces. Names, the app ID and the old-name migration live in `src/util.ts`.
- TypeScript style: no semicolons, 2-space indent, double quotes, trailing commas, ~80 columns (prettier defaults with `semi: false`). Nix: alejandra.
- GTK/GLib come from `gi://` imports; gnim's lowercase JSX tags are registered in `src/jsx.ts` (add new ones there).
- Keep comments sparse: explain why, not what.
- Known traps (details in the README): keep `await app.runAsync(…)`, not `app.run(…)`; each window is created inside gnim's `createRoot`; the package builds its own GdkPixbuf `loaders.cache` and wraps by hand (`dontWrapGApps`) so WebP thumbnails work; during a scan the auto-selection is kept on the first image.
- The app ID is `io.github.Rido_o.Vitrine`; data lives in `~/.cache/vitrine/thumbnails` and `~/.local/state/vitrine/history`. Changing names or paths needs a migration like the existing `OLD_APP_NAME` one.
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
