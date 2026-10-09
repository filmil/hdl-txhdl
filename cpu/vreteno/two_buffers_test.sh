#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# The flagship's GL icosahedron on two harts draws every frame into the
# buffer that is not on the screen (issue 1551). ico_gl_hdmi runs on the
# machine with both harts, Razboj a stand-in that is done at once, and
# `--rings` says for every ring of the doorbell the rows its tile
# table's tiles start at and where the scanout showed from as it came.
# A frame's rows from 512 down are the second buffer, 0x4220_0000; above
# them, the first, 0x4200_0000. Every tiled ring must draw into one
# buffer, never the one shown, and both buffers must be drawn.
#
# Both harts drawing into the second buffer drew there while it was
# shown, every other frame, and left the first buffer to the logo.
#
#   two_buffers_test.sh <machine> <ico_gl_hdmi_bin.bin>
set -eu
machine="$1"
image="$2"
out=$(mktemp)
"$machine" --image "$PWD/$image" --at 0x40000000 --steps 16000000 \
  --rings >"$out" 2>&1 || true
grep -E '^ico|^ring' "$out"
fail=0
grep -q '^ico two harts$' "$out" ||
  { echo "FAIL: the program did not start hart 1" >&2; fail=1; }
awk '
  /^ring [0-9]+: 0x8/ {
    lo = $5; hi = $7; shown = $9
    drawn = (lo >= 512) ? 1 : 0
    if ((hi >= 512) != drawn) {
      printf "FAIL: %s draws into both buffers\n", $0 > "/dev/stderr"; bad = 1
    }
    on = (shown == "0x42200000") ? 1 : (shown == "0x42000000") ? 0 : -1
    if (on < 0) {
      printf "FAIL: %s shows from neither buffer\n", $0 > "/dev/stderr"; bad = 1
    }
    if (drawn == on) {
      printf "FAIL: %s draws into the buffer on the screen\n", $0 > "/dev/stderr"
      bad = 1
    }
    seen[drawn] = 1; n++
  }
  END {
    if (n < 8) { printf "FAIL: %d frames rung, wanted 8\n", n > "/dev/stderr"; bad = 1 }
    if (!seen[0] || !seen[1]) { print "FAIL: a buffer never drawn" > "/dev/stderr"; bad = 1 }
    exit bad
  }' "$out" || fail=1
[ "$fail" -eq 0 ] && echo "PASS: every frame into the buffer not shown, both drawn"
exit "$fail"
