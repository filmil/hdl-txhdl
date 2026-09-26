#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# The register headers the drivers include are the maps the parts
# declare (issue 709). They are committed under include/vreteno/regs,
# since a reader's own `west build` of this module runs no Bazel, and
# this test holds every one to what //tools/regmap writes from its
# declaration now: a map with no header, a header with no map, or a
# header that differs fails, naming the fix.
#
#   bazel run //zephyr:regs_update   # writes them again
set -eu
regmap="$1"
dir="$2"
fail=0
maps=$("$regmap" list | cut -d: -f1)
for m in $maps; do
  if [ ! -f "$dir/$m.h" ]; then
    echo "no header for the map \`$m\`: $dir/$m.h"
    fail=1
    continue
  fi
  if ! "$regmap" "$m" c | diff -u "$dir/$m.h" - ; then
    echo "$dir/$m.h is not what the map \`$m\` declares"
    fail=1
  fi
done
for h in "$dir"/*.h; do
  [ -e "$h" ] || continue
  m=$(basename "$h" .h)
  if ! printf '%s\n' $maps | grep -qx "$m"; then
    echo "$h has no map; //tools/regmap lists: $(echo $maps)"
    fail=1
  fi
done
if [ "$fail" -ne 0 ]; then
  echo "run: bazel run //zephyr:regs_update"
  exit 1
fi
echo "every map has its header, as declared: $(echo $maps)"
