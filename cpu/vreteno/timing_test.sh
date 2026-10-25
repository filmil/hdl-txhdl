#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# The machine model's timing mode against the board's own programs
# (issue 1392). cpi.rs's warm loop and ethperf.rs's copies into a DDR3
# slot are run on the model with `--timing --no-dcache`, and the cycles
# they report must be what the calibrated costs give. The board's
# figures, beside them, are what the costs were fitted to, on a board
# with no data cache yet (7ae6d1bf came before 7b9e9482):
#
#   cpi dmem warm   7036 on the board, 7002 here (-0.5%)
#   cpi ddr3 warm   7088 on the board, 7002 here (-1.2%)
#   tx copy 1514    19926 on the board, 20144 here (+1.1%)
#   tx copy 60      840 on the board, 852 here (+1.4%)
#
# Each loop's exit is a backward branch that falls through, which costs
# its wrong guess, 2 cycles (#1752).
#
# The machine charges the data cache by default since #1561, and the
# board has had one since 7b9e9482, so `--no-dcache` here holds the
# costs to the fit they came from rather than to today's board. The
# cached model does not fit the copies yet: today's board copies a
# frame in 4578 cycles and 60 bytes in 206 (the nightly of 10-23,
# main 2a501c4a), where the model gives 6474 and 283 (#1918).
#
# A change to the costs moves these, which is what this test is for: it
# says so, and the comment above is updated with the new fit.
#
#   timing_test.sh <machine> <cpi_ram_bin.bin> <ethperf_ram_bin.bin>
set -eu
machine="$1"
cpi="$2"
ethperf="$3"
out=$(mktemp)
fail=0
"$machine" --image "$PWD/$cpi" --at 0x40000000 --steps 20000000 --timing \
  --no-dcache >"$out" 2>&1 || true
cat "$out"
for want in "cpi dmem warm 7002 3004" "cpi ddr3 warm 7002 3004"; do
  grep -q "^$want\$" "$out" || { echo "FAIL: no '$want'" >&2; fail=1; }
done
"$machine" --image "$PWD/$ethperf" --at 0x40000000 --steps 40000000 \
  --timing --no-dcache >"$out" 2>&1 || true
grep "^tx" "$out"
grep -qE "^tx copy 1514 .* cycles/frame 20144\$" "$out" ||
  { echo "FAIL: tx copy 1514 is not 20144 cycles a frame" >&2; fail=1; }
grep -qE "^tx copy 60 .* cycles/frame 852\$" "$out" ||
  { echo "FAIL: tx copy 60 is not 852 cycles a frame" >&2; fail=1; }
# The default, with the data cache, is pinned too, so that a change to
# it says so: cpi 7092 warm, and the copies 6474 and 283, which today's
# board does in 4578 and 206 (#1918).
"$machine" --image "$PWD/$cpi" --at 0x40000000 --steps 20000000 --timing \
  >"$out" 2>&1 || true
grep "^cpi" "$out"
for want in "cpi dmem warm 7092 3004" "cpi ddr3 warm 7092 3004"; do
  grep -q "^$want\$" "$out" ||
    { echo "FAIL: cached, no '$want'" >&2; fail=1; }
done
"$machine" --image "$PWD/$ethperf" --at 0x40000000 --steps 40000000 \
  --timing >"$out" 2>&1 || true
grep "^tx" "$out"
grep -qE "^tx copy 1514 .* cycles/frame 6474\$" "$out" ||
  { echo "FAIL: cached, tx copy 1514 is not 6474 cycles a frame" >&2; fail=1; }
grep -qE "^tx copy 60 .* cycles/frame 283\$" "$out" ||
  { echo "FAIL: cached, tx copy 60 is not 283 cycles a frame" >&2; fail=1; }
[ "$fail" -eq 0 ] && echo "PASS: the timing mode gives the calibrated cycles"
exit "$fail"
