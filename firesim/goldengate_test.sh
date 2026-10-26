#!/usr/bin/env bash
# Runs Golden Gate on FireSim's GCD example for the Nitefury II platform and
# checks that the Verilog and the host driver's header come out.
#
# usage: goldengate_test.sh <goldengate> <GCD.fir> <GCD.anno.json>
set -euo pipefail
gg=$1 fir=$2 anno=$3
out=${TEST_TMPDIR:-$(mktemp -d)}/gg
mkdir -p "$out"
"$gg" -i "$fir" -faf "$anno" -td "$out" \
    -ggcp firesim.midasexamples -ggcs DefaultNitefuryConfig \
    --output-filename-base FireSim-generated \
    --allow-unrecognized-annotations --no-dedup > "$out/goldengate.log" 2>&1 || {
    tail -40 "$out/goldengate.log" >&2
    exit 1
}
for f in FireSim-generated.sv FireSim-generated.const.h; do
    [ -s "$out/$f" ] || { echo "Golden Gate wrote no $f" >&2; exit 1; }
done
# The simulator's top module and GCD's own logic, in the Verilog.
grep -q "^module FPGATop" "$out/FireSim-generated.sv" || { echo "no FPGATop in the Verilog" >&2; exit 1; }
grep -q "GCDDUT" "$out/FireSim-generated.sv" || { echo "no GCDDUT in the Verilog" >&2; exit 1; }
echo "Golden Gate: $(wc -c < "$out/FireSim-generated.sv") bytes of Verilog"
