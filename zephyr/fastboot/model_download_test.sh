#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# A fastboot download to the fastboot server on the machine model (issue
# 1390): smoltcp's TCP/IP and a small fastboot client on the far end of
# the cable send 256 KiB, through the model's Ethernet port, which paces
# and drops frames as the board's does. The test passes when the server
# answers OKAY and the staged bytes are the image's, and it prints the
# steps a byte, the client's retransmits and the port's drops, so a
# change to the receive path has a number before a board session.
#
# A step is an instruction: the model has no memory latency, so this
# counts the work the path does and how TCP fares on the port, and the
# board still says what the DDR3 costs.
#
#   model_download_test.sh <machine> <the image's files...>
set -eu
machine="$1"
shift
image=
for f in "$@"; do
  case "$f" in
  *.bin) image="$f" ;;
  esac
done
[ -n "$image" ] || { echo "FAIL: no .bin among $*" >&2; exit 1; }
out=$(mktemp)
"$machine" --image "$PWD/$image" --at 0x40000000 --steps 60000000 \
  --fastboot-peer 262144 >"$out" 2>&1 || true
cat "$out"
fail=0
grep -q "^fastboot: 262144 bytes in" "$out" ||
  { echo "FAIL: the server never answered OKAY" >&2; fail=1; }
grep -q "the staged image is intact" "$out" ||
  { echo "FAIL: the staged bytes are not the image" >&2; fail=1; }
if grep -q "halted (Fault" "$out"; then
  echo "FAIL: the model faulted" >&2
  fail=1
fi
[ "$fail" -eq 0 ] && echo "PASS: $(grep '^fastboot: 262144' "$out")"
exit "$fail"
