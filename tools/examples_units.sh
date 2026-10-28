#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# The last row of the showcase's units table, "The examples' own units":
# the lowered units the examples define, the files that hold them, and
# how many of those files reach a netlist simulation (#1997).
#
# A unit is an `impl .. Unit .. for` item under `#[lower]` in
# lib/examples, outside the *_test.rs files, which are tests of the
# lowering and not examples. A file reaches a simulation when it is a
# source file of a `_vsim` test's dependencies, as stream_alu.rs is
# through ex_stream_alu. Run from the repository root; it needs bazel
# for the last count, and fails rather than reports a guess.
set -euo pipefail
list=$(git grep -l -E '^[[:space:]]*#\[lower\]' -- 'lib/examples/*.rs' |
  grep -v '_test\.rs$' || true)
units=""
for f in $list; do
  # A lowered item's header runs from the attribute to the first `{`,
  # with comments and other attributes on their own lines left out; it
  # is a unit when it is an impl whose trait, just before `for`, is
  # Unit or Unit<..>.
  units+=$(awk '/^[[:space:]]*#\[lower\]/ { h = ""; on = 1; next }
    on && /^[[:space:]]*(\/\/|#\[)/ { next }
    on { h = h " " $0
      if (index($0, "{")) {
        on = 0
        if (h ~ /^[[:space:]]*impl/ &&
            h ~ /(^|[^A-Za-z0-9_])Unit(<.*>)?[[:space:]]+for[[:space:]]/)
          print FILENAME
      } }' "$f")$'\n'
done
count() { grep -c . || true; }
nunits=$(printf '%s' "$units" | count)
files=$(printf '%s' "$units" | sort -u)
nfiles=$(printf '%s\n' "$files" | count)
sources=$(bazel query \
  'kind("source file", deps(attr(name, "_vsim", kind(".*_test", //...))))' \
  | sed -n 's|^//lib/examples:||p')
nsim=0
for f in $files; do
  if printf '%s\n' "$sources" | grep -qx "$(basename "$f")"; then
    nsim=$((nsim + 1))
  fi
done
echo "$nunits units, in $nfiles files, $nsim of them simulated"
