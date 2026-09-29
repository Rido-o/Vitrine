#!/usr/bin/env bash
# Builds the benchmark corpus (see README.md): ~3,000 images in nested
# folders, derived from the images in SOURCE_DIR (a handful of large photos
# or wallpapers is enough). Run from the repo root:
#   bench/make-corpus.sh SOURCE_DIR [OUT_DIR]
# OUT_DIR defaults to ~/.cache/vitrine-bench/corpus. Needs ~5 GB of disk and
# ~1.2 GB of RAM per job (JOBS, default 4). Deterministic
# for the same sources, so runs on different days compare.
set -euo pipefail

src=${1:?usage: make-corpus.sh SOURCE_DIR [OUT_DIR]}
out=${2:-${XDG_CACHE_HOME:-$HOME/.cache}/vitrine-bench/corpus}

if [[ -z ${IN_NIX_SHELL_CORPUS:-} ]]; then
  IN_NIX_SHELL_CORPUS=1 exec nix shell --inputs-from . nixpkgs#imagemagick \
    nixpkgs#exiftool nixpkgs#findutils -c "$0" "$@"
fi

mapfile -t sources < <(find "$src" -maxdepth 1 -type f \
  \( -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.png' -o -iname '*.webp' \) |
  sort)
((${#sources[@]} > 0)) || { echo "no images in $src" >&2; exit 1; }

# kind count width height extension
kinds=(
  "huge 50 8000 6000 jpg"
  "large 250 6000 4000 jpg"
  "medium 700 4000 3000 jpg"
  "small 1300 2560 1600 jpg"
  "webp 250 3840 2160 webp"
  "alpha 200 1920 1080 png"
  "portrait 250 4000 3000 jpg"
  "gif 10 960 540 gif"
)

mkdir -p "$out"
jobs=$out/.jobs
: > "$jobs"
index=0
for kind in "${kinds[@]}"; do
  read -r name count width height ext <<< "$kind"
  for ((i = 0; i < count; i++)); do
    source=${sources[index % ${#sources[@]}]}
    # Nested folders a0..a9/b0..b4, with every 7th image at the top.
    if ((index % 7 == 0)); then dir=$out; else
      dir=$out/a$((index % 10))/b$((index / 10 % 5))
    fi
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "$width" "$height" \
      "$((index * 37 % 360))" "$source" "$dir/$name-$(printf %04d "$index").$ext" \
      >> "$jobs"
    index=$((index + 1))
  done
done

make_one() {
  IFS=$'\t' read -r name width height hue source target <<< "$1"
  [[ -e $target ]] && return
  mkdir -p "$(dirname "$target")"
  # Written under a temporary name, so an interrupted run leaves no truncated
  # file for the next one to skip.
  part=${target%.*}.part.${target##*.}
  # Crop to the size, shift the hue so every file differs.
  common=(-resize "${width}x${height}^" -gravity center
    -extent "${width}x${height}" -modulate "100,100,$((100 + hue % 40 - 20))")
  case $name in
    alpha) magick "$source" "${common[@]}" -alpha set -background none \
      -rotate 8 -extent "${width}x${height}" "$part" ;;
    # ImageMagick's -orient doesn't write the EXIF tag; 6 is "rotate 90° CW".
    portrait) magick "$source" "${common[@]}" -quality 85 "$part" &&
      exiftool -q -n -Orientation=6 -overwrite_original "$part" ;;
    # 12 frames turning 30° each, 80 ms apart.
    gif)
      frames=$(mktemp -d)
      for i in $(seq 0 11); do
        magick "$source" "${common[@]}" -distort SRT "$((i * 30))" \
          "$frames/$(printf %02d "$i").png"
      done
      magick -delay 8 -loop 0 "$frames"/*.png "$part"
      rm -rf "$frames"
      ;;
    *) magick "$source" "${common[@]}" -quality 85 "$part" ;;
  esac
  mv "$part" "$target"
}
export -f make_one

# A 48 MP image takes ~1.2 GB in ImageMagick; one job per core exhausted 15 GB.
parallel=${JOBS:-4}
export MAGICK_MEMORY_LIMIT=1.5GiB
echo "Generating $index images in $out ($parallel at a time) ..."
tr '\n' '\0' < "$jobs" | xargs -0 -P "$parallel" -I{} bash -c 'make_one "$1"' _ {}
rm "$jobs"
echo "Done: $(find "$out" -type f | wc -l) files, $(du -sh "$out" | cut -f1)"
