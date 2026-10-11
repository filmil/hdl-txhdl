#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# The canonical teapot on two harts shows each frame for about as long
# as the next, counted as the board counts (#1639). It runs on the
# machine in the timing mode with the board's 60 Hz raster, a frame
# every 1,668,321 cycles, and the scanout taking its base as each
# vertical blanking begins; `--flips` says every base it took and the
# frame it took it in. After the first four flips, while the harts
# settle, each frame must stay on the screen within one blanking of half
# its pair's time, which is two of each other.
#
# Before #1639 the frames went up in pairs, back to back: on this raster
# one of each pair for one blanking and the other for thirty, as the
# board showed one for one and the other for thirty-five.
#
#   flip_pacing_test.sh <machine> <demo_teapot_1_utah_bin.bin>
set -eu
machine="$1"
image="$2"
out=$(mktemp)
trap 'rm -f "$out"' EXIT
"$machine" --image "$PWD/$image" --at 0x40000000 --steps 200000000 \
  --timing --dcache --board-raster --flips >"$out" 2>&1 || true
grep -E '^ico|^teapot|^flip' "$out"
fail=0
grep -q '^ico two harts$' "$out" ||
  { echo "FAIL: the program did not start hart 1" >&2; fail=1; }
awk '
  /^flip [0-9]+:/ { at[n++] = $6 }
  END {
    if (n < 12) { printf "FAIL: %d flips, wanted 12\n", n > "/dev/stderr"; exit 1 }
    for (i = 4; i + 2 < n; i++) {
      a = at[i + 1] - at[i]; b = at[i + 2] - at[i + 1]
      if (a - b > 2 || b - a > 2) {
        printf "FAIL: flip %d shown %d blankings, the next %d\n", i, a, b > "/dev/stderr"
        bad = 1
      }
    }
    exit bad
  }' "$out" || fail=1
[ "$fail" -eq 0 ] && echo "PASS: each frame shown about as long as the next, in the board's blankings"
exit "$fail"
