#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Runs the Ethernet test on the machine the board's port is cabled to,
# which .bazelrc names in TXHDL_BOARD_SERVER: uploads the tester, one
# static binary, and runs it there over ssh under a timeout.
#
#   run [--server=HOST] [--interface=fpga-a200t-eth0] [--count=64]
#       [--size=64] [--seconds=10]
#
# The tester opens a raw socket, which needs either root or
# `cap_net_raw` on the uploaded binary. The link has to be up. Both are
# one command each on that machine, and the README of //eth says which.
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
interface=fpga-a200t-eth0
count=64
size=64
seconds=10
for a in "$@"; do
  case "$a" in
    --server=*) server="${a#*=}" ;;
    --interface=*) interface="${a#*=}" ;;
    --count=*) count="${a#*=}" ;;
    --size=*) size="${a#*=}" ;;
    --seconds=*) seconds="${a#*=}" ;;
    *) echo "unknown argument: $a" >&2; exit 2 ;;
  esac
done
[[ -n "$server" ]] || {
  echo "--server=HOST is required, or TXHDL_BOARD_SERVER in the environment" >&2
  exit 2
}

tester="$(rlocation _main/eth/hil/ethtest_/ethtest)"
dir='~/txhdl_ethtest'
# The binary is uploaded only when it differs from the one already
# there. A file's capabilities belong to the file, so `setcap
# cap_net_raw+ep` would have to be run again after every upload, and
# that needs a password.
want="$(sha256sum "$tester" | cut -d' ' -f1)"
have="$(ssh -o BatchMode=yes "$server" \
  "mkdir -p $dir; sha256sum $dir/ethtest 2>/dev/null | cut -d' ' -f1" || true)"
if [[ "$want" != "$have" ]]; then
  echo "[ethtest] uploading the tester"
  ssh -o BatchMode=yes "$server" "rm -f $dir/ethtest"
  scp -q -o BatchMode=yes "$tester" "$server:$dir/ethtest"
  # The capability belongs to the file, so a new upload has none. The
  # board server allows this one command without a password, for this
  # one path, in /etc/sudoers.d/60-fpga-hil.
  ssh -o BatchMode=yes "$server" \
    "sudo -n setcap cap_net_raw+ep \$HOME/txhdl_ethtest/ethtest" ||
    echo "[ethtest] could not grant the capability; run setcap there by hand"
fi
ssh -o BatchMode=yes "$server" \
  "timeout --signal=TERM --kill-after=5 $((seconds + 20)) $dir/ethtest '$interface' '$count' '$size' '$seconds'"
