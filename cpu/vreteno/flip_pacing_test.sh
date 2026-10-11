#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# The GL icosahedron on two harts shows each frame for about as long as
# the next (#1639). ico_gl_hdmi runs on the machine with both harts, and
# `--flips` says every base the scanout was given and the stand-in
# raster's frame it came in. Once the harts have settled, after the
# first eight flips, each frame must stay on the screen within one
# raster frame of half its pair's time, which is two frames of each
# other.
#
# Before #1639 the two harts' frames went up in pairs, back to back: one
# of each pair for one raster frame and the other for ten.
#
#   flip_pacing_test.sh <machine> <ico_gl_hdmi_bin.bin>
set -eu
machine="$1"
image="$2"
out=$(mktemp)
trap 'rm -f "$out"' EXIT
"$machine" --image "$PWD/$image" --at 0x40000000 --steps 60000000 \
  --flips >"$out" 2>&1 || true
grep -E '^ico|^flip' "$out"
fail=0
grep -q '^ico two harts$' "$out" ||
  { echo "FAIL: the program did not start hart 1" >&2; fail=1; }
awk '
  /^flip [0-9]+:/ { at[n++] = $6 }
  END {
    if (n < 24) { printf "FAIL: %d flips, wanted 24\n", n > "/dev/stderr"; exit 1 }
    for (i = 8; i + 2 < n; i++) {
      a = at[i + 1] - at[i]; b = at[i + 2] - at[i + 1]
      if (a - b > 2 || b - a > 2) {
        printf "FAIL: flip %d shown %d raster frames, the next %d\n", i, a, b > "/dev/stderr"
        bad = 1
      }
    }
    exit bad
  }' "$out" || fail=1
[ "$fail" -eq 0 ] && echo "PASS: each frame shown about as long as the next"
exit "$fail"
