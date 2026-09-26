#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
#
# One SymbiYosys run over one Verilog file, on the pinned Yosys and,
# for a bounded run, the pinned z3: the proof, the bounded search or
# the cover of a unit's `check!`, `assume!` and `cover!` statements
# (issue 581). defs.bzl says what calls it.
#
#   formal.sh VERILOG TOP MODE DEPTH EXPECT SBY PYTHON [LOG [TRACE]]
#
# MODE is prove, bmc or cover. A proof runs `abc pdr`, which is inside
# yosys-abc and needs no solver, with `aigsmt none` so that a failure
# is reported as such (exit code 2) rather than as a trace it cannot
# build (16). A bounded run and a cover run `smtbmc z3` to DEPTH
# steps, which give a trace: the counterexample, or the path to each
# cover point.
#
# The script exits 0 only when SymbiYosys's exit code is EXPECT: 0 for
# a pass, 2 for a failure. Anything else fails, and in particular an
# UNKNOWN (4) never passes, since induction at a small depth answers
# UNKNOWN for a real bug.
#
# LOG, when given, receives the run's log with its timestamps and its
# timings removed, so that a document can include it and the build
# stays reproducible. TRACE, when given, receives the trace the run
# wrote, as a VCD, or an empty file when there was none.
set -euo pipefail

verilog=$1 top=$2 mode=$3 depth=$4 expect=$5 sby=$6 python=$7
log=${8:-} trace=${9:-}
root=$PWD

# The two Debian trees, found by a file each holds: under the runfiles
# of a test, or under the execroot of a build action.
find_tree() {
  local marker=$1 found=""
  for base in "${RUNFILES_DIR:-}" "$root/../.." "$root"; do
    [ -n "$base" ] || continue
    found=$(find -L "$base" -path "*$marker" -print -quit 2>/dev/null || true)
    [ -n "$found" ] && break
  done
  [ -n "$found" ] || {
    echo "no $marker under the runfiles or the execroot" >&2
    exit 1
  }
  echo "${found%"$marker"}"
}
ytree=$(find_tree /tree/usr/bin/yosys)/tree
ztree=$(find_tree /tree/usr/bin/z3)/tree

# Yosys's SMT helpers are Python scripts that name /usr/bin/python3 and
# look for their modules in /usr/share/yosys, neither of which is where
# the tree is. Each gets a wrapper that runs it on the hermetic Python
# with the tree's module directory on the path.
work=${TEST_TMPDIR:-$(mktemp -d)}/formal_$$
mkdir -p "$work/bin"
for tool in yosys-smtbmc yosys-witness; do
  cat > "$work/bin/$tool" <<WRAP
#!/bin/bash
export PYTHONPATH=$ytree/usr/share/yosys
exec "$root/$python" "$ytree/usr/bin/$tool" "\$@"
WRAP
  chmod +x "$work/bin/$tool"
done

export YOSYS=$ytree/usr/bin/yosys
export ABC=$ytree/usr/bin/yosys-abc
export SMTBMC=$work/bin/yosys-smtbmc
export WITNESS=$work/bin/yosys-witness
export PATH="$ztree/usr/bin:$work/bin:$PATH"
libs=$ytree/usr/lib/x86_64-linux-gnu:$ytree/lib/x86_64-linux-gnu
export LD_LIBRARY_PATH="$libs:$ztree/usr/lib/x86_64-linux-gnu"
export HOME=$work

case "$mode" in
  prove)
    options="mode prove
aigsmt none"
    engines="abc pdr" ;;
  bmc | cover)
    options="mode $mode
depth $depth"
    engines="smtbmc z3" ;;
  *) echo "mode is prove, bmc or cover, not $mode" >&2; exit 1 ;;
esac
cp "$verilog" "$work/$top.v"
cat > "$work/$top.sby" <<CONF
[options]
$options

[engines]
$engines

[script]
read -formal $top.v
prep -top $top

[files]
$top.v
CONF

cd "$work"
rc=0
"$root/$sby" -f "$top.sby" > sby.log 2>&1 || rc=$?
grep -E 'summary|DONE' sby.log || true

if [ -n "$log" ]; then
  # What the run found, without the clock, the durations, the
  # commands and the paths, which differ from run to run: the engine's
  # own lines on what it checked and proved, and the summary.
  sed -E -e 's/^SBY [0-9:]+ //' -e 's/^\[[^]]*\] //' \
    -e 's/^engine_0: ## +[0-9:]+ +/engine_0: /' \
    -e '/Elapsed|Time = |sec \(|Warning: The last|VarMax/d' \
    -e '/Frame Clauses|^engine_0: [0-9]+ : |Block =|Counter-example is not/d' \
    -e '/Removing directory|Copy '"'"'|starting process|ABC command line/d' \
    -e '/finished \(returncode|Writing trace to (Verilog|constraints|Yosys)/d' \
    sby.log > "$root/$log"
fi
if [ -n "$trace" ]; then
  found=$(find "$top" -name 'trace*.vcd' -print -quit 2>/dev/null || true)
  if [ -n "$found" ]; then
    cp "$found" "$root/$trace"
  else
    : > "$root/$trace"
  fi
fi

if [ "$rc" != "$expect" ]; then
  echo "expected SymbiYosys to exit with $expect, it exited with $rc;" \
    "the log follows" >&2
  cat sby.log >&2
  exit 1
fi
echo "SymbiYosys exited with $rc, as expected"
