#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Zephyr's GL program draws every frame into the buffer that is not on
# the screen, and shows each frame it drew (#1605), as two_buffers_test
# holds the bare-metal icosahedron to (issue 1551). The image runs on
# the machine, Razboj a stand-in that is done at once, and `--rings`
# says for every ring of the doorbell the rows its list draws in and
# where the scanout showed from as it came. Rows from 512 down are the
# second buffer, 0x4220_0000; above them, the first, 0x4200_0000. Every
# ring must draw into one buffer, never the one shown, the next ring
# must find that buffer shown, and both buffers must be drawn. Only the
# first ring may come before anything is shown.
#
#   two_buffers_test.sh <machine> <the image's files...>
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
"$machine" --image "$PWD/$image" --at 0x40000000 --steps 8000000 \
  --rings >"$out" 2>&1 || true
grep -E '^gles|^ring' "$out"
fail=0
grep -q '^gles egl 1\.4' "$out" ||
  { echo "FAIL: EGL did not come up" >&2; fail=1; }
awk '
  /^ring [0-9]+:/ {
    i = $2 + 0; lo = $5; hi = $7; shown = $9
    if ($4 != "rows") {
      printf "FAIL: %s says no rows\n", $0 > "/dev/stderr"; bad = 1; next
    }
    drawn = (lo >= 512) ? 1 : 0
    if ((hi >= 512) != drawn) {
      printf "FAIL: %s draws into both buffers\n", $0 > "/dev/stderr"; bad = 1
    }
    on = (shown == "0x42200000") ? 1 : (shown == "0x42000000") ? 0 : -1
    if (on < 0 && !(i == 0 && shown == "none")) {
      printf "FAIL: %s shows from neither buffer\n", $0 > "/dev/stderr"; bad = 1
    }
    if (drawn == on) {
      printf "FAIL: %s draws into the buffer on the screen\n", $0 > "/dev/stderr"
      bad = 1
    }
    if (n > 0 && on != last) {
      printf "FAIL: %s, the frame before was not shown\n", $0 > "/dev/stderr"
      bad = 1
    }
    last = drawn; seen[drawn] = 1; n++
  }
  END {
    if (n < 8) { printf "FAIL: %d frames rung, wanted 8\n", n > "/dev/stderr"; bad = 1 }
    if (!seen[0] || !seen[1]) { print "FAIL: a buffer never drawn" > "/dev/stderr"; bad = 1 }
    exit bad
  }' "$out" || fail=1
[ "$fail" -eq 0 ] && echo "PASS: every frame into the buffer not shown, then shown"
exit "$fail"
