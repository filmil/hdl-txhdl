#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# The GL program boots on the machine model and draws (issue 1287):
# Zephyr's start, the copy of its data into the core's own memory, EGL's
# setup, and sixty frames, with its stacks and its state in that memory.
# What Razboj draws is for the board, since the model's Razboj finishes
# a list at once.
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
"$machine" --image "$PWD/$image" --at 0x40000000 --steps 60000000 \
  --until "gles frame 60 " >"$out" 2>&1 || true
cat "$out"
fail=0
grep -q "Booting Zephyr OS" "$out" ||
  { echo "FAIL: Zephyr did not start" >&2; fail=1; }
grep -q "gles egl 1.4" "$out" ||
  { echo "FAIL: EGL was not set up" >&2; fail=1; }
grep -q "gles frame 60 " "$out" ||
  { echo "FAIL: the program did not draw sixty frames" >&2; fail=1; }
if grep -q "halted (Fault" "$out"; then
  echo "FAIL: the model faulted" >&2
  fail=1
fi
[ "$fail" -eq 0 ] && echo "PASS: the GL program draws on the model"
exit "$fail"
