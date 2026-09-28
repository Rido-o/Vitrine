#!/usr/bin/env bash
# Runs the benchmark (see README.md) for one app and prints its RESULT lines.
# Run from the repo root:
#   bench/run.sh ts|rs [cold|warm] [CORPUS_DIR]
# cold (the default) starts with an empty thumbnail cache; warm reuses the
# previous run's. Uses a headless sway unless BENCH_DISPLAY names a Wayland
# display to use instead (e.g. to watch it).
set -euo pipefail

app=${1:?usage: run.sh ts|rs [cold|warm] [CORPUS_DIR]}
mode=${2:-cold}
bench=${XDG_CACHE_HOME:-$HOME/.cache}/vitrine-bench
corpus=${3:-$bench/corpus}
[[ -d $corpus ]] || { echo "no corpus at $corpus (bench/make-corpus.sh)" >&2; exit 1; }

case $app in
  ts) out=$(nix build --no-link --print-out-paths .#bench-ts); bin=$out/bin/vitrine ;;
  rs) out=$(nix build --no-link --print-out-paths .#spike); bin=$out/bin/vitrine-rs ;;
  *) echo "app must be ts or rs" >&2; exit 1 ;;
esac

# Each app gets its own cache and state, away from the real ones.
home=$bench/home-$app
[[ $mode == cold ]] && rm -rf "$home/cache"
mkdir -p "$home/cache" "$home/state"

config=
# By its config file: nixpkgs' sway wrapper runs the real sway as a child.
cleanup() {
  [[ -n $config ]] || return 0
  pkill -f -- "-c $config" || true
  rm -f "$config"
}
trap cleanup EXIT

display=${BENCH_DISPLAY:-}
if [[ -z $display ]]; then
  runtime=${XDG_RUNTIME_DIR:?}
  before=$(ls "$runtime" | grep -E '^wayland-[0-9]+$' || true)
  config=$(mktemp)
  echo 'output HEADLESS-1 resolution 1920x1080@60Hz' > "$config"
  sway=$(nix build --no-link --print-out-paths --inputs-from . nixpkgs#sway)/bin/sway
  WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WAYLAND_DISPLAY= \
    "$sway" -c "$config" > /dev/null 2>&1 &
  for _ in $(seq 50); do
    display=$(comm -13 <(echo "$before") \
      <(ls "$runtime" | grep -E '^wayland-[0-9]+$') | head -1)
    [[ -n $display ]] && break
    sleep 0.1
  done
  [[ -n $display ]] || { echo "headless sway didn't start" >&2; exit 1; }
fi

echo "# $app $mode $(date -Iseconds) $(git rev-parse --short HEAD)"
XDG_CACHE_HOME=$home/cache XDG_STATE_HOME=$home/state WAYLAND_DISPLAY=$display \
  VITRINE_PROBE=1 "$bin" "$corpus" 2>&1 | grep "^RESULT" | sed "s/^RESULT //"
