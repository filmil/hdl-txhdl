#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Sends a program to the loader in the board's boot memory, from the
# machine .bazelrc names in TXHDL_BOARD_SERVER: uploads the sender and
# the image and runs the sender there over ssh, under a timeout.
#
#   load [--server=HOST] [--port=/dev/ttyUSB0] [--baud=115200]
#        [--address=0x40000000] [--seconds=30] [--reset] --image=PATH
#
# With --reset the sender first holds the serial line low for 30 ms, a
# zero byte at 300 baud, which the board's top turns into a reset of
# the core, so the loader is back whatever program had the core, and no
# reprogram is needed.
set -euo pipefail

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
port=/dev/ttyUSB0
baud=115200
address=0x40000000
seconds=30
image=""
reset=""
for a in "$@"; do
  case "$a" in
    --server=*) server="${a#*=}" ;;
    --port=*) port="${a#*=}" ;;
    --baud=*) baud="${a#*=}" ;;
    --address=*) address="${a#*=}" ;;
    --seconds=*) seconds="${a#*=}" ;;
    --image=*) image="${a#*=}" ;;
    --reset) reset="reset" ;;
    *) echo "unknown argument: $a" >&2; exit 2 ;;
  esac
done
[[ -n "$server" ]] || {
  echo "--server=HOST is required, or TXHDL_BOARD_SERVER in the environment" >&2
  exit 2
}
[[ -n "$image" ]] || { echo "--image=PATH is required" >&2; exit 2; }

sender="$(rlocation _main/cpu/vreteno/board/remote/load_bin_/load_bin)"
dir='~/txhdl_load'
ssh -o BatchMode=yes "$server" "mkdir -p $dir && rm -f $dir/load $dir/image.bin"
scp -q -o BatchMode=yes "$sender" "$server:$dir/load"
scp -q -o BatchMode=yes "$image" "$server:$dir/image.bin"
ssh -o BatchMode=yes "$server" \
  "timeout --signal=TERM --kill-after=5 $((seconds + 20)) $dir/load '$port' '$baud' '$address' $dir/image.bin '$seconds' $reset"
