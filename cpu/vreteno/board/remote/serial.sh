#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Watches the board's serial port on the machine it is attached to,
# for a while, printing what comes, and types a reply once the board
# has said a line; the watcher, one static binary, is uploaded and
# runs there over ssh, under a timeout that frees the port whatever
# happens here.
#
#   serial [--server=HOST] [--port=/dev/ttyUSB0] [--baud=115200]
#          [--seconds=30] [--reply=]
#
# The server is TXHDL_BOARD_SERVER from the environment, which .bazelrc
# sets for `bazel run`, unless --server says otherwise.
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
seconds=30
reply=""
for a in "$@"; do
  case "$a" in
    --server=*) server="${a#*=}" ;;
    --port=*) port="${a#*=}" ;;
    --baud=*) baud="${a#*=}" ;;
    --seconds=*) seconds="${a#*=}" ;;
    --reply=*) reply="${a#*=}" ;;
    *) echo "unknown argument: $a" >&2; exit 2 ;;
  esac
done
[[ -n "$server" ]] || {
  echo "--server=HOST is required, or TXHDL_BOARD_SERVER in the environment" >&2
  exit 2
}
watcher="$(rlocation _main/cpu/vreteno/board/remote/serial_bin_/serial_bin)"
dir='~/txhdl_hw_server'
# The last upload is read-only, as the build made it; it goes first.
ssh -o BatchMode=yes "$server" "mkdir -p $dir && rm -f $dir/serial"
scp -q -o BatchMode=yes "$watcher" "$server:$dir/serial"
ssh -o BatchMode=yes "$server" \
  "timeout --signal=TERM --kill-after=5 $((seconds + 10)) $dir/serial '$port' '$baud' '$seconds' '$reply'"
