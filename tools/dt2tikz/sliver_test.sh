#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# A bus value's text is written only where it fits (issue 865).
#
# The trace is one 16-bit bus: ffff from tick 0, 1234 for the one tick
# at 10, abcd for twenty ticks from 11, and 0 from 31. Drawn from tick 9
# into 6 cm, ffff is a sliver of one tick at the window's start and 1234
# a sliver of one tick inside it, and neither has room for four
# characters; abcd and 0 have plenty. Drawn into 40 cm, a tick is wide
# enough for four characters, and 1234 is written again.
set -euo pipefail

dt2tikz="$1"
dir="${TEST_TMPDIR:-$(mktemp -d)}"
trace="$dir/sliver.dt"
cat > "$trace" <<'EOF'
v=1111111111111111.
..........
v=0001001000110100.
.
v=1010101111001101.
....................
v=0000000000000000.
....................
EOF

fail=0
check() {
  if ! eval "$2"; then
    echo "FAIL: $1"
    fail=1
  fi
}

# Every value whose text is drawn has room for it: the bottom line
# drawn before a node spans the interval between its crossovers, which
# are 0.07 cm each, and a character takes 0.13 cm.
all_fit() {
  awk '
    /^\\draw/ && / -- / && !/rectangle/ {
      n = split($0, f, /[(),]/)
      if (n >= 6 && f[3] == f[6]) { a = f[2]; b = f[5] }
    }
    /^\\node at/ {
      s = substr($0, index($0, "{") + 1)
      s = substr(s, 1, length(s) - 2)
      if (0.13 * length(s) > b - a + 0.14 + 0.005) {
        print "  " s " in " (b - a + 0.14) " cm"; bad = 1
      }
    }
    END { exit bad }' "$1"
}

narrow="$dir/narrow.tex"
"$dt2tikz" "$trace" --from 9 --width 6 > "$narrow"
check "the sliver at the window's start is not written" \
  "! grep -q '{ffff}' '$narrow'"
check "the sliver inside the window is not written" \
  "! grep -q '{1234}' '$narrow'"
check "a value with room is written" "grep -q '{abcd}' '$narrow'"
check "the last value is written" "grep -q '{0}' '$narrow'"
check "every value written fits, pressed into 6 cm" "all_fit '$narrow'"

wide="$dir/wide.tex"
"$dt2tikz" "$trace" --from 9 --width 40 > "$wide"
check "a tick of 40 cm's drawing has room for 1234" \
  "grep -q '{1234}' '$wide'"
check "every value written fits, in 40 cm" "all_fit '$wide'"

if [ "$fail" -ne 0 ]; then
  echo "--- 6 cm:"; cat "$narrow"
  exit 1
fi
echo "PASS"
