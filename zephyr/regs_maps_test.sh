#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# The register headers are kept by write_source_files, which compares
# the committed directory with the one //tools/regmap writes (issues
# 1179 and 1191). This is the backstop beside it: every map regmap
# lists has its header committed, and every header committed is a map
# it lists, so either kind of drift fails naming the fix.
#
#   regs_maps_test.sh <regmap> <header dir>
set -eu
regmap="$1"
dir="$2"
declared=$("$regmap" list | cut -d: -f1 | sort)
fail=0
for m in $declared; do
  if [ ! -f "$dir/$m.h" ]; then
    echo "no header for the map \`$m\`: $dir/$m.h; run"
    echo "  bazel run //zephyr:regs_update"
    fail=1
  fi
done
for h in "$dir"/*.h; do
  [ -e "$h" ] || continue
  m=$(basename "$h" .h)
  if ! printf '%s\n' $declared | grep -qx "$m"; then
    echo "$h is committed and //tools/regmap declares no map \`$m\`; run"
    echo "  bazel run //zephyr:regs_update"
    fail=1
  fi
done
[ "$fail" -eq 0 ] || exit 1
echo "a header for every map declared: $(echo $declared)"
