#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# The register headers are kept by write_source_files, which checks
# each header REG_MAPS in zephyr/BUILD.bazel names (issue 1179). This
# holds that list to the maps //tools/regmap declares and to the
# headers committed, so a map added to the parts, or a header left
# behind by one removed, fails naming the fix rather than going
# unchecked.
#
#   regs_maps_test.sh <regmap> <header dir> <REG_MAPS...>
set -eu
regmap="$1"
dir="$2"
shift 2
listed=$(printf '%s\n' "$@" | sort)
declared=$("$regmap" list | cut -d: -f1 | sort)
fail=0
for m in $declared; do
  if ! printf '%s\n' $listed | grep -qx "$m"; then
    echo "the map \`$m\` is declared and not in REG_MAPS: add it to"
    echo "  REG_MAPS in zephyr/BUILD.bazel, then run"
    echo "  bazel run //zephyr:regs_update"
    fail=1
  fi
done
for m in $listed; do
  if ! printf '%s\n' $declared | grep -qx "$m"; then
    echo "REG_MAPS names \`$m\`, which //tools/regmap does not list:"
    echo "  remove it from REG_MAPS and delete $dir/$m.h"
    fail=1
  fi
done
for m in $listed; do
  if [ ! -f "$dir/$m.h" ]; then
    echo "no header for the map \`$m\`: $dir/$m.h; run"
    echo "  bazel run //zephyr:regs_update"
    fail=1
  fi
done
for h in "$dir"/*.h; do
  [ -e "$h" ] || continue
  m=$(basename "$h" .h)
  if ! printf '%s\n' $listed | grep -qx "$m"; then
    echo "$h is committed and REG_MAPS does not name \`$m\`"
    fail=1
  fi
done
[ "$fail" -eq 0 ] || exit 1
echo "REG_MAPS is every map declared: $(echo $declared)"
