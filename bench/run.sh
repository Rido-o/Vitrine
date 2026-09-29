#!/usr/bin/env bash
# Runs the benchmark (see README.md) for one app and prints its RESULT lines.
# Run from the repo root:
#   bench/run.sh ts|rs [cold|warm] [CORPUS_DIR]
# cold (the default) starts with an empty thumbnail cache; warm reuses the
# previous run's. Uses a headless sway unless BENCH_DISPLAY names a Wayland
# display to use instead (e.g. to watch it). BENCH_WRAP prefixes the app's
# command, e.g. BENCH_WRAP="perf record -g -o /tmp/perf.data".
# BENCH_LOG=FILE keeps the app's whole output.
set -euo pipefail

app=${1:?usage: run.sh ts|rs [cold|warm] [CORPUS_DIR]}
mode=${2:-cold}
bench=${XDG_CACHE_HOME:-$HOME/.cache}/vitrine-bench
corpus=${3:-$bench/corpus}
# The headless display, e.g. BENCH_OUTPUT=3840x2160@144Hz BENCH_SCALE=1.5.
output=${BENCH_OUTPUT:-1920x1080@60Hz}
scale=${BENCH_SCALE:-1}
hz=${output##*@}
hz=${hz%Hz}
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
  echo "output HEADLESS-1 resolution $output scale $scale" > "$config"
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
  # For the spike's hold_key: a virtual keyboard (GTK repeats held keys).
  VITRINE_PROBE_WTYPE=$(nix build --no-link --print-out-paths \
    --inputs-from . nixpkgs#wtype)/bin/wtype
  export VITRINE_PROBE_WTYPE
fi

# BENCH_REAL_CACHE=1 uses the app's real cache and state instead (e.g. to run
# on a real folder whose thumbnails exist); mode is then ignored.
if [[ -n ${BENCH_REAL_CACHE:-} ]]; then
  export XDG_CACHE_HOME=${XDG_CACHE_HOME:-$HOME/.cache}
  export XDG_STATE_HOME=${XDG_STATE_HOME:-$HOME/.local/state}
else
  export XDG_CACHE_HOME=$home/cache XDG_STATE_HOME=$home/state
fi

echo "# $app $mode $output scale $scale $(date -Iseconds) $(git rev-parse --short HEAD)"
WAYLAND_DISPLAY=$display \
  VITRINE_PROBE=1 VITRINE_PROBE_HZ=$hz ${BENCH_WRAP:-} "$bin" "$corpus" 2>&1 |
  tee "${BENCH_LOG:-/dev/null}" | grep "^RESULT" | sed "s/^RESULT //"
