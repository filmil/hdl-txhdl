#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# A value is drawn at the tick its timestamp names (issue 880).
#
# sqlite2drawtiming writes a dot before every timestamp, the first one
# included, and the timestamp as a comment. A clock high at timestamp 0
# and low at 1 is drawn high from tick 0, at x = 0, and the axis's 0 is
# at the clock's first rise; counted by its dots alone it was drawn from
# tick 1.
set -euo pipefail

dt2tikz="$1"
dir="${TEST_TMPDIR:-$(mktemp -d)}"
trace="$dir/clock.dt"
cat > "$trace" <<'TRACE'
.
# timestamp: 0
clk=1.
.
# timestamp: 1
clk=0.
.
# timestamp: 2
clk=1.
.
# timestamp: 3
clk=0.
TRACE

out="$dir/clock.tex"
"$dt2tikz" "$trace" > "$out"

fail=0
# The first high stretch starts at x = 0.00, and the clock rises again
# two ticks later, where the axis says 2.
if ! grep -q '^\\draw\[black\] (0\.00,0\.55) -- ' "$out"; then
  echo "FAIL: the clock is not high from tick 0"
  fail=1
fi
rise2=$(grep -E '^\\draw\[black\] \(([0-9.]+),-?0\.00\) -- \(\1,0\.55\);' "$out" | head -1 | sed -E 's/^\\draw\[black\] \(([0-9.]+),.*/\1/')
label2=$(grep -E 'font=\\tiny\] at \(([0-9.]+),[-0-9.]+\) \{2\};' "$out" | sed -E 's/.* at \(([0-9.]+),.*/\1/')
if [ -z "$rise2" ] || [ "$rise2" != "$label2" ]; then
  echo "FAIL: the second rise is at x = ${rise2:-none}, the axis's 2 at x = ${label2:-none}"
  fail=1
fi
if [ "$fail" -ne 0 ]; then
  cat "$out"
  exit 1
fi
echo "PASS"
