#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# The nightly regression of the flagship (issue 1854): one bitstream,
# the images of this commit, the same checks every night, and what
# moved since the night before. Run it from a checkout of the commit
# the images are for, holding the board token:
#
#   bazel run //cpu/vreteno/board/remote:nightly -- --bit=FILE \
#       [--out=DIR] [--prev=RESULTS] [--server=HOST] [--dry-run]
#
# The board server is --server, or TXHDL_BOARD_SERVER, which .bazelrc
# sets for every `bazel run`; a board run with neither stops. Paths
# are the caller's, relative to where `bazel run` was called.
#
# In order: the bitstream programmed, checked against its own sha256;
# Linux through fastboot, timed from `booting` to OpenSBI and to
# `/init`, with the fastboot app's own rows from the same load; the
# canonical demos, by demos.sh, each to its end, with its tx_dropped;
# the Ethernet port, its link from the PHY's registers, a hundred pings
# from the board server while Linux is up, and ethperf's transmit and
# receive, `ethtest send` answering each `rx waiting` with a paced
# burst (#1955); and the costs
# image. It writes DIR/<start>-<sha>/: a log per check, results.tsv
# (check, metric, value, unit, status) and diff.tsv against the night
# before, the newest results.tsv in DIR before this one, or --prev.
#
# --dry-run touches no board: it builds every image, and runs on
# //cpu/vreteno:machine what has a model there, Linux to /init, each
# serial demo for a while, and the costs image, and marks the rest
# `skipped`. Like demos.sh it refuses to cross 07:05-07:15 UTC, when the
# board is switched off, and checks that nothing holds the serial port.
set -euo pipefail
trap 'echo "nightly: stopped at line $LINENO: $BASH_COMMAND" >&2' ERR

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
out="${TMPDIR:-/tmp}/txhdl-nightly"
prev=""
bit=""
dry=""
# A path as the caller meant it: `bazel run` starts in the runfiles,
# and the caller's own directory is BUILD_WORKING_DIRECTORY.
abspath() {
  case "$1" in
    /*) echo "$1" ;;
    *) echo "${BUILD_WORKING_DIRECTORY:-$PWD}/$1" ;;
  esac
}
for a in "$@"; do
  case "$a" in
    --server=*) server="${a#*=}" ;;
    --out=*) out="$(abspath "${a#*=}")" ;;
    --prev=*) prev="$(abspath "${a#*=}")" ;;
    --bit=*) bit="$(abspath "${a#*=}")" ;;
    --dry-run) dry=1 ;;
    *) echo "unknown argument: $a" >&2; exit 2 ;;
  esac
done
ws="${BUILD_WORKSPACE_DIRECTORY:?run this with bazel run}"
lib="$(rlocation txhdl/cpu/vreteno/board/remote/nightly_lib.sh)"
# shellcheck source=nightly_lib.sh
source "$lib"
load="$(rlocation txhdl/cpu/vreteno/board/remote/load)"
demos="$(rlocation txhdl/cpu/vreteno/board/remote/demos)"
list="$(rlocation txhdl/docs/demos.tsv)"
if [[ -z "$dry" ]]; then
  [[ -n "$server" ]] || { echo "--server=HOST or TXHDL_BOARD_SERVER" >&2; exit 2; }
  [[ -f "$bit" ]] || { echo "--bit=FILE: the bitstream to program" >&2; exit 2; }
fi
port=/dev/ttyUSB0
fbdir='~/txhdl_fastboot'
lock=/data/cache/vivado-install/vivado.lock

# Not across the daily power-off: the run takes about an hour.
now=$(date -u +%s)
day=$(date -u -d "$(date -u +%F)" +%s)
need=3600
if [[ -z "$dry" ]]; then
  for edge in $((day + 7*3600 + 300)) $((day + 31*3600 + 300)); do
    if (( now < edge + 600 && now + need > edge )); then
      echo "a run of ${need}s now would cross 07:05-07:15 UTC; wait" >&2
      exit 1
    fi
  done
fi

cd "$ws"
sha="$(git rev-parse --short=8 HEAD)"
# A dry run says so in its name, and is never a board run's night
# before.
run="$(TZ=America/Los_Angeles date +%Y%m%dT%H%M%z)-$sha${dry:+-dry}"
dir="$out/$run"
mkdir -p "$out"
if ! mkdir "$dir"; then
  echo "cannot make $dir: a run this minute at this commit, or see above" >&2
  exit 1
fi
exec > >(tee -a "$dir/run.log") 2>&1
echo "nightly at $sha into $dir${dry:+ (dry run, on the model)}"
results="$dir/results.tsv"
printf 'check\tmetric\tvalue\tunit\tstatus\n' >"$results"

# The receive tester too, so that no build lands in the board's 40 s
# wait for a burst.
images=(//linux:board_boot //linux:board_demo_boot //zephyr:fastboot
  //cpu/vreteno/rust:phyregs_ram_bin //cpu/vreteno/rust:ethperf_ram_bin
  //cpu/vreteno/rust:costs_ram_bin //eth/hil:run)
model=(//cpu/vreteno:machine //linux:model_boot)
bazel build "${images[@]}" ${dry:+"${model[@]}"}
file() { bazel cquery --output=files "$1" 2>/dev/null | grep -m1 '\.bin$\|/machine$'; }
img() { echo "$ws/$(file "$1")"; }
ethtest="$ws/$(bazel cquery --output=files //eth/hil:run 2>/dev/null | grep -m1 '^bazel-out/')"
stamp() { while IFS= read -r l; do printf '%s %s\n' "$(date -u +%T.%3N)Z" "$l"; done; }

# --- On the model ------------------------------------------------------

if [[ -n "$dry" ]]; then
  machine="$(img //cpu/vreteno:machine)"
  # A run of the model on an image at the board's DDR3, its console
  # stamped like the board's, until it says `until` or runs its steps.
  model_run() {
    local image="$1" log="$2" until="$3" steps="$4"; shift 4
    timeout 1800 "$machine" --image "$image" --at 0x40000000 \
      --steps "$steps" ${until:+--until "$until"} "$@" </dev/null 2>&1 |
      stamp >"$log" || true
  }
  echo "linux on the model"
  model_run "$(img //linux:model_boot)" "$dir/linux.log" \
    "Run /init as init process" 250000000
  if grep -aq "Run /init as init process" "$dir/linux.log"; then
    row linux "reached /init (model)" yes - pass >>"$results"
  else
    row linux "reached /init (model)" no - fail >>"$results"
  fi
  while IFS=$'\t' read -r name target how seconds _ _; do
    [[ "$how" == serial ]] || {
      row "demo $name" ran "-" - "skipped" >>"$results"
      continue
    }
    echo "demo $name on the model"
    bazel build "$target" >/dev/null 2>&1
    model_run "$(img "$target")" "$dir/$name.log" "" 20000000
    # The demo's own last line, not the machine's report of where it
    # stopped; none unless the demo said its name, `demo ...`, which
    # each canonical demo does first.
    last=""
    if grep -aqE '^\S+ demo ' "$dir/$name.log"; then
      last="$(cut -c15- "$dir/$name.log" | grep -avE '^(stopped |hart [0-9]+:)' |
        grep -av '^[[:space:]]*$' | tail -1 || true)"
    fi
    read_demo "$name" "$dir/$name.log" "$last" >>"$results"
  done < <(grep -v '^#' "$list" | grep -v '^[[:space:]]*$')
  # The rows a board run has and the model cannot, in the board run's
  # order, so that the two read alike.
  for m in link "1000 full" "ping lost" ethperf; do
    row ethernet "$m" - - skipped >>"$results"
  done
  row fastboot "load to listening" - - skipped >>"$results"
  row fastboot "getvar answered" - - skipped >>"$results"
  echo "costs on the model"
  model_run "$(img //cpu/vreteno/rust:costs_ram_bin)" "$dir/costs.log" "" \
    200000000 --timing
  read_costs "$dir/costs.log" >>"$results"
fi

# --- On the board ------------------------------------------------------

if [[ -z "$dry" ]]; then
  holders() { ssh -n -o BatchMode=yes "$server" "fuser $port 2>/dev/null" || true; }
  release() {
    ssh -n -o BatchMode=yes "$server" "for p in \$(fuser $port 2>/dev/null); do
        case \"\$(tr '\\0' ' ' </proc/\$p/cmdline)\" in
          */txhdl_load/load\ $port*) kill \$p \$(ps -o ppid= -p \$p) ;;
        esac; done" || true
  }
  program() {
    echo "programming $(sha256sum "$bit" | cut -c1-16)"
    flock -o -w 5400 "$lock" bazel run //flagship:flagship_prog_given -- \
      --bitfile="$bit" --sha256="$(sha256sum "$bit" | cut -c1-64)" \
      --hostport localhost:3122 --device "*/xilinx_tcf/Digilent/*"
  }
  free_port() {
    if [[ -n "$(holders)" ]]; then
      echo "the serial port is held on $server; stopping" >&2
      exit 1
    fi
  }
  # A program through the serial loader, its console for `seconds`.
  serial_run() {
    local image="$1" log="$2" seconds="$3"
    free_port
    "$load" --server="$server" --reset --image="$image" \
      --seconds="$seconds" </dev/null 2>&1 | stamp >"$log" || true
    release
  }
  # The fastboot app through the serial loader, `getvar product` asked
  # of it, then an image booted, and its console kept until `until` or
  # `seconds`. The reader stays up, so the system stays up, until
  # fb_end. Sets `answered`.
  fb_boot() {
    local image="$1" log="$2" until="$3" seconds="$4"
    free_port
    ssh -n -o BatchMode=yes "$server" \
      "mkdir -p $fbdir/nightly && rm -f $fbdir/nightly/image.bin"
    scp -q -o BatchMode=yes "$image" "$server:$fbdir/nightly/image.bin"
    ( "$load" --server="$server" --reset --image="$(img //zephyr:fastboot)" \
        --seconds=$((seconds + 600)) </dev/null 2>&1 | stamp >"$log" ) &
    reader=$!
    for _ in $(seq 1 150); do
      grep -q 'listening on port' "$log" 2>/dev/null && break
      sleep 1
    done
    sender="$(holders)"
    answered=no
    # No app listening, no boot: the rows say so, and the run goes on.
    if ! grep -q 'listening on port' "$log" 2>/dev/null; then
      echo "  the fastboot app never listened"
      return
    fi
    if timeout 30 ssh -n -o BatchMode=yes "$server" \
        "cd $fbdir && ./fastboot -s tcp:192.168.1.50 getvar product" >>"$log" 2>&1; then
      answered=yes
    fi
    timeout 300 ssh -n -o BatchMode=yes "$server" \
      "cd $fbdir && ./fastboot -s tcp:192.168.1.50 boot nightly/image.bin" >>"$log" 2>&1 || true
    for _ in $(seq 1 "$seconds"); do
      grep -aqF "$until" "$log" && break
      sleep 1
    done
  }
  fb_end() {
    [[ -n "${sender:-}" ]] && ssh -n -o BatchMode=yes "$server" "kill $sender" 2>/dev/null || true
    release
    kill "$reader" 2>/dev/null || true
    wait "$reader" 2>/dev/null || true
  }

  program
  echo "linux through fastboot"
  fb_boot "$(img //linux:board_boot)" "$dir/linux.log" "Run /init as init process" 120
  fb_end
  read_linux linux "$dir/linux.log" >>"$results"
  # The fastboot app's rows come from this load, and go in after the
  # Ethernet port's, where the order puts them.
  fastboot_rows="$(read_fastboot "$dir/linux.log" "$answered")"

  echo "the demos"
  "$demos" --server="$server" --bit="$bit" --out="$dir/demos" || true
  manifest="$(ls -d "$dir"/demos/*/manifest.tsv 2>/dev/null | tail -1)"
  if [[ -n "$manifest" ]]; then
    while IFS=$'\t' read -r name _ _ _ _ last; do
      read_demo "$name" "$(dirname "$manifest")/$name.log" "$last" >>"$results"
    done < <(tail -n +2 "$manifest")
  else
    row demos manifest - - fail >>"$results"
  fi

  echo "the ethernet port"
  program
  serial_run "$(img //cpu/vreteno/rust:phyregs_ram_bin)" "$dir/phyregs.log" 15
  read_phy "$dir/phyregs.log" >>"$results"
  # No reprogram: phyregs answers the serial line's reset, so the
  # fastboot app loads after it.
  fb_boot "$(img //linux:board_demo_boot)" "$dir/ping-linux.log" "txhdl: demo done" 180
  timeout 60 ssh -n -o BatchMode=yes "$server" "ping -c 100 -i 0.2 192.168.1.50" \
    >"$dir/ping.log" 2>&1 || true
  fb_end
  read_ping "$dir/ping.log" >>"$results"
  # Linux does not answer the serial line's reset, so the part is
  # programmed again before ethperf.
  program
  free_port
  ( "$load" --server="$server" --reset --image="$(img //cpu/vreteno/rust:ethperf_ram_bin)" \
      --seconds=300 </dev/null 2>&1 | stamp >"$dir/ethperf.log" ) &
  reader=$!
  # A burst from the board server at each `rx waiting N`, by the tester
  # issue 1038 wrote for it; the board waits 40 s for the first frame.
  answered_waits=0
  for _ in $(seq 1 300); do
    waits="$(grep -ac 'rx waiting' "$dir/ethperf.log" 2>/dev/null || true)"
    while (( answered_waits < waits )); do
      answered_waits=$((answered_waits + 1))
      size="$(grep -a 'rx waiting' "$dir/ethperf.log" | sed -n "${answered_waits}p" | awk '{print $NF}')"
      # Built above; run as it is, with no Bazel in the board's wait,
      # and with its own runfiles rather than the nightly's (#1898).
      sent=0
      # Paced, so that the count is the board's and not the server's
      # send rate (#1955).
      [[ -n "$size" ]] && { BUILD_WORKSPACE_DIRECTORY="$ws" bare "$ethtest" \
        --server="$server" --mode=send --count=1000 --size="$size" \
        --gap-us="$(rx_gap_us "$size")" \
        >>"$dir/ethtest.log" 2>&1 || sent=$?; }
      # Exit 2 is the sender unable to start at all, no tester or a bad
      # argument, which no later burst will mend: say so and stop
      # waiting, and the receive rows fail as they read (#1898).
      if (( sent == 2 )); then
        echo "[nightly] the ethperf sender cannot run; see ethtest.log" >&2
        break 2
      fi
    done
    (( answered_waits >= 4 )) && grep -aq 'rx copy 1514' "$dir/ethperf.log" && break
    sleep 1
  done
  release
  kill "$reader" 2>/dev/null || true
  wait "$reader" 2>/dev/null || true
  read_ethperf "$dir/ethperf.log" >>"$results"
  [[ -n "$fastboot_rows" ]] && printf '%s\n' "$fastboot_rows" >>"$results"

  echo "the costs image"
  # No reprogram: ethperf answers the serial line's reset.
  serial_run "$(img //cpu/vreteno/rust:costs_ram_bin)" "$dir/costs.log" 60
  read_costs "$dir/costs.log" >>"$results"
fi

# --- What moved --------------------------------------------------------

if [[ -z "$prev" ]]; then
  # The newest earlier night of the same kind, by when its results were
  # written rather than by its name, whose Pacific offset changes with
  # daylight saving: a board run against a board run, a dry run against
  # a dry run.
  prev="$(ls -dt "$out"/*/results.tsv 2>/dev/null | grep -v "^$dir/" |
    if [[ -n "$dry" ]]; then grep -- '-dry/'; else grep -v -- '-dry/'; fi |
    head -1 || true)"
fi
if [[ -n "$prev" && -f "$prev" ]]; then
  diff_results "$prev" "$results" >"$dir/diff.tsv"
  echo "against $prev"
else
  echo "no night before to compare with"
fi
cat >"$dir/README.md" <<EOF
# TxHDL nightly, $run

main at $(git rev-parse HEAD), bitstream $( (sha256sum "$bit" 2>/dev/null || echo none) | cut -c1-16)${dry:+, a dry run on the model}.
results.tsv has a row per check; diff.tsv compares it with ${prev:-nothing}.
EOF
echo "done: $dir"
column -t -s $'\t' "$results"
if [[ -f "$dir/diff.tsv" ]]; then
  echo
  grep -E 'moved|gone' "$dir/diff.tsv" | column -t -s $'\t' || echo "no status moved"
fi
