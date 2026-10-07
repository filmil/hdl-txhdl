#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# The fastboot server boots on the machine model to its listening line
# (issue 1386): Zephyr's start, the copy of its data into the core's own
# memory, its drivers, the entropy source among them, which the model
# lacked, and the network stack's start. What it does on the network is
# for the board, or for a peer that speaks TCP.
#
#   model_test.sh <machine> <the image's files...>
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
"$machine" --image "$PWD/$image" --at 0x40000000 --steps 5000000 >"$out" 2>&1 || true
cat "$out"
fail=0
grep -q "Booting Zephyr OS" "$out" ||
  { echo "FAIL: Zephyr did not start" >&2; fail=1; }
grep -q "fastboot: listening on port 5554" "$out" ||
  { echo "FAIL: the server never listened" >&2; fail=1; }
if grep -q "halted (Fault" "$out"; then
  echo "FAIL: the model faulted" >&2
  fail=1
fi
[ "$fail" -eq 0 ] && echo "PASS: fastboot listens on the model"
exit "$fail"
