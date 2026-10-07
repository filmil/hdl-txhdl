#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# The machine model's timing mode against the board's own programs
# (issue 1392). cpi.rs's warm loop and ethperf.rs's copies into a DDR3
# slot are run on the model with `--timing`, and the cycles they report
# must be what the calibrated costs give. The board's figures, beside
# them, are what the costs were fitted to:
#
#   cpi dmem warm   7036 on the board, 7000 here (-0.5%)
#   cpi ddr3 warm   7088 on the board, 7000 here (-1.2%)
#   tx copy 1514    19926 on the board, 20142 here (+1.1%)
#   tx copy 60      840 on the board, 850 here (+1.2%)
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
  >"$out" 2>&1 || true
cat "$out"
for want in "cpi dmem warm 7000 3004" "cpi ddr3 warm 7000 3004"; do
  grep -q "^$want\$" "$out" || { echo "FAIL: no '$want'" >&2; fail=1; }
done
"$machine" --image "$PWD/$ethperf" --at 0x40000000 --steps 40000000 \
  --timing >"$out" 2>&1 || true
grep "^tx" "$out"
grep -qE "^tx copy 1514 .* cycles/frame 20142\$" "$out" ||
  { echo "FAIL: tx copy 1514 is not 20142 cycles a frame" >&2; fail=1; }
grep -qE "^tx copy 60 .* cycles/frame 850\$" "$out" ||
  { echo "FAIL: tx copy 60 is not 850 cycles a frame" >&2; fail=1; }
[ "$fail" -eq 0 ] && echo "PASS: the timing mode gives the calibrated cycles"
exit "$fail"
