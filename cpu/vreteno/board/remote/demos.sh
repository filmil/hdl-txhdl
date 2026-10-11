#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Runs the canonical demos on the board, in the order docs/demos.tsv
# gives, for the seconds it gives, and keeps what a video needs to be
# cut from them (issue 1592). Run it from a checkout of the commit the
# demos are for, holding the board token:
#
#   bazel run //cpu/vreteno/board/remote:demos -- [--out=DIR] [--dry-run]
#
# It builds the flagship and every image at this commit, programs the
# flagship, and for each demo: checks that nothing else holds the
# board's serial port, loads the demo with the serial loader or with
# fastboot, keeps the console for the demo's seconds with every line
# stamped in UTC, then stops its own reader on the board server. It
# writes DIR/<start>-<sha>/: a log per demo, manifest.tsv with each
# demo's image, sha256 and stage times, and README.md. The recording
# is the board server's: its owner cuts <demo>.mp4 from the stage
# times in manifest.tsv into the same directory on the share.
#
# It refuses to run across 07:05-07:15 UTC, when the board is switched
# off every day, and a demo after a fastboot one starts from a
# reprogram, since Linux, and a program booted by fastboot that takes
# over the serial port, need not answer the serial line's reset.
set -euo pipefail
# Say where it stopped, if it stops: the first run stopped after its
# last demo without a word, and a run that does not say what failed
# cannot be mended.
trap 'echo "demos: stopped at line $LINENO: $BASH_COMMAND" >&2' ERR

# --- begin runfiles.bash initialization v3 ---
f=bazel_tools/tools/bash/runfiles/runfiles.bash
# shellcheck disable=SC1090
source "${RUNFILES_DIR:-/dev/null}/$f" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "${RUNFILES_MANIFEST_FILE:-/dev/null}" | cut -f2- -d' ')" 2>/dev/null || \
  source "$0.runfiles/$f" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "$0.runfiles_manifest" | cut -f2- -d' ')" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "$0.exe.runfiles_manifest" | cut -f2- -d' ')" 2>/dev/null || \
  { echo>&2 "ERROR: cannot find $f"; exit 1; }; f=
# --- end runfiles.bash initialization v3 ---

server="${TXHDL_BOARD_SERVER:-}"
out="${TMPDIR:-/tmp}/txhdl-demos"
dry=""
for a in "$@"; do
  case "$a" in
    --server=*) server="${a#*=}" ;;
    --out=*) out="${a#*=}" ;;
    --dry-run) dry=1 ;;
    *) echo "unknown argument: $a" >&2; exit 2 ;;
  esac
done
[[ -n "$server" ]] || { echo "--server=HOST or TXHDL_BOARD_SERVER" >&2; exit 2; }
ws="${BUILD_WORKSPACE_DIRECTORY:?run this with bazel run}"
list="$(rlocation _main/docs/demos.tsv)"
load="$(rlocation _main/cpu/vreteno/board/remote/load)"
port=/dev/ttyUSB0
fbdir='~/txhdl_fastboot'
lock=/data/cache/vivado-install/vivado.lock

# The demos, comments and the header left out.
mapfile -t demos < <(grep -v '^#' "$list" | grep -v '^[[:space:]]*$')
(( ${#demos[@]} > 0 )) || { echo "no demos in $list" >&2; exit 1; }

# Not across the daily power-off: about two minutes a demo, with the
# programming, must end before 07:05 UTC or start after 07:15.
now=$(date -u +%s)
day=$(date -u -d "$(date -u +%F)" +%s)
need=$(( 300 + 150 * ${#demos[@]} ))
for edge in $((day + 7*3600 + 300)) $((day + 31*3600 + 300)); do
  if (( now < edge + 600 && now + need > edge )); then
    echo "a run of ${need}s now would cross 07:05-07:15 UTC; wait" >&2
    exit 1
  fi
done

cd "$ws"
sha="$(git rev-parse --short=8 HEAD)"
# The run is named by its start in Pacific time, ISO 8601 basic format
# with the offset, which has no colons: 20261010T1830-0700-94d3c50d.
run="$(TZ=America/Los_Angeles date +%Y%m%dT%H%M%z)-$sha"
dir="$out/$run"
mkdir -p "$dir"
# Everything the run says goes to its directory as well.
exec > >(tee -a "$dir/run.log") 2>&1
echo "demos at $sha into $dir"

# Every image and the fastboot app, built at this commit; the flagship
# too, under the Vivado lock, unless this is a dry run.
targets=(//zephyr:fastboot)
for d in "${demos[@]}"; do targets+=("$(cut -f2 <<<"$d")"); done
bazel build "${targets[@]}"
if [[ -z "$dry" ]]; then
  flock -o -w 14400 "$lock" bazel build //flagship:flagship_pnr
fi
file() { bazel cquery --output=files "$1" 2>/dev/null | grep -m1 '\.\(bin\|bit\)$'; }
fastboot_bin="$ws/$(file //zephyr:fastboot)"
bit="$ws/$(file //flagship:flagship_pnr || true)"
printf 'demo\timage\tsha256\tstart\tend\tlast line\n' >"$dir/manifest.tsv"

# Whatever holds the serial port on the board server: none, or the
# pids of the senders.
holders() { ssh -n -o BatchMode=yes "$server" "fuser $port 2>/dev/null" || true; }
# Stops this tool's own senders on the board server, each with the
# timeout above it, after checking each is a sender.
release() {
  ssh -n -o BatchMode=yes "$server" "for p in \$(fuser $port 2>/dev/null); do
      case \"\$(tr '\\0' ' ' </proc/\$p/cmdline)\" in
        */txhdl_load/load\ $port*) kill \$p \$(ps -o ppid= -p \$p) ;;
      esac; done" || true
}
program() {
  echo "programming $( (sha256sum "$bit" 2>/dev/null || echo none) | cut -c1-16)"
  [[ -n "$dry" ]] && return
  flock -o -w 5400 "$lock" bazel run //flagship:flagship_prog -- \
    --hostport localhost:3122 --device "*/xilinx_tcf/Digilent/*"
}
stamp() { while IFS= read -r l; do printf '%s %s\n' "$(date -u +%T.%3N)Z" "$l"; done; }

program
after_fastboot=""
for d in "${demos[@]}"; do
  IFS=$'\t' read -r name target how seconds until extra <<<"$d"
  image="$ws/$(file "$target")"
  sum="$(sha256sum "$image" | cut -c1-64)"
  log="$dir/$name.log"
  [[ -n "$after_fastboot" ]] && program
  if [[ -n "$(holders)" ]]; then
    echo "$name: the serial port is held on $server; stopping" >&2
    exit 1
  fi
  echo "$name: $target ($how, ${seconds}s)"
  [[ -n "$dry" ]] && continue
  if [[ "$how" == serial ]]; then
    "$load" --server="$server" --reset --image="$image" \
      --seconds="$seconds" </dev/null 2>&1 | stamp >"$log" || true
    start="$(grep -am1 ' ok 40000000' "$log" | cut -c1-13 || true)"
    after_fastboot=""
  else
    # The fastboot app, then the image and its extra files from the
    # board server, then the demo's seconds or its last line.
    # A Bazel output is read-only, and so is the copy an earlier run
    # left, which an upload cannot overwrite: it goes first.
    ssh -n -o BatchMode=yes "$server" \
      "mkdir -p $fbdir/demos && rm -f $fbdir/demos/$name.bin"
    scp -q -o BatchMode=yes "$image" "$server:$fbdir/demos/$name.bin"
    ( "$load" --server="$server" --reset --image="$fastboot_bin" \
        --seconds=$((seconds + 120)) </dev/null 2>&1 | stamp >"$log" ) &
    reader=$!
    for _ in $(seq 1 150); do
      if grep -q 'listening on port' "$log" 2>/dev/null; then break; fi
      sleep 1
    done
    # The sender holding the port for this demo, stopped by its pid when
    # the demo's time is up.
    sender="$(holders)"
    files="demos/$name.bin"
    if [[ "$extra" != - ]]; then files="$files $extra"; fi
    timeout 300 ssh -n -o BatchMode=yes "$server" \
      "cd $fbdir && ./fastboot -s tcp:192.168.1.50 boot $files" >>"$log" 2>&1 || true
    start="$(grep -am1 'fastboot: booting' "$log" | cut -c1-13 || true)"
    for _ in $(seq 1 "$seconds"); do
      if [[ "$until" != - ]] && grep -aqF "$until" "$log"; then break; fi
      sleep 1
    done
    if [[ -n "$sender" ]]; then
      ssh -n -o BatchMode=yes "$server" "kill $sender" 2>/dev/null || true
    fi
    release
    kill "$reader" 2>/dev/null || true
    wait "$reader" 2>/dev/null || true
    after_fastboot=1
  fi
  release
  end="$(date -u +%T.%3N)Z"
  last="$(grep -av '^\S* \[load\]' "$log" | tail -1 | cut -c15- | tr '\t' ' ' || true)"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "$target" "$sum" "${start:-?}" "$end" "$last" \
    >>"$dir/manifest.tsv"
  echo "  $name: from ${start:-?} to $end"
done

cat >"$dir/README.md" <<EOF
# TxHDL demos, $run

main at $(git rev-parse HEAD), flagship $( (sha256sum "$bit" 2>/dev/null || echo none) | cut -c1-16), run $(date -u +%FT%TZ).
The stage times in manifest.tsv are UTC; each demo's clip is cut from
its start to its end, as <demo>.mp4.
EOF
echo "done: $dir"
cat "$dir/manifest.tsv"
