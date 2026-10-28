#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# The last row of the showcase's units table, "The examples' own units":
# the lowered units the examples define, the files that hold them, and
# how many of those files reach a netlist simulation (#1997).
#
# A unit is an `impl .. Unit .. for` item under `#[lower]` in
# lib/examples, outside the *_test.rs files, which are tests of the
# lowering and not examples. A file reaches a simulation when its
# example's binary, ex_<file> or <file> itself, is a dependency of a
# `_vsim` test, as stream_alu.rs is through ex_stream_alu. Run from the
# repository root; it needs bazel for the last count.
set -eu
unit_files() {
  for f in $(git grep -l '^#\[lower\]' -- 'lib/examples/*.rs' |
    grep -v '_test\.rs$'); do
    # A lowered item's header runs from the line after the attribute to
    # the first `{`; it is a unit when it is an impl of Unit.
    awk '/^#\[lower\]/ { h = ""; on = 1; next }
      on { h = h " " $0
        if (index($0, "{")) {
          on = 0
          if (h ~ /^ *impl/ && h ~ /Unit.* for /) print FILENAME
        } }' "$f"
  done
}
units=$(unit_files | wc -l)
files=$(unit_files | sort -u)
nfiles=$(printf '%s\n' "$files" | wc -l)
simulated=$(bazel query \
  'kind("rust_binary", deps(attr(name, "_vsim", kind(".*_test", //...))))' \
  2>/dev/null | sed -n 's|^//lib/examples:||p')
nsim=0
for f in $files; do
  b=$(basename "$f" .rs)
  if printf '%s\n' "$simulated" | grep -qx -e "$b" -e "ex_$b"; then
    nsim=$((nsim + 1))
  fi
done
echo "$units units, in $nfiles files, $nsim of them simulated"
