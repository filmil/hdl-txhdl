#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Writes include/vreteno/regs/<map>.h for every map //tools/regmap
# lists, from its declaration, into the working tree, and removes a
# header whose map is gone (issue 709). `//zephyr:regs_test` fails
# until this has been run after a map changes.
set -eu
regmap=$(realpath "$1")
dir="$BUILD_WORKSPACE_DIRECTORY/zephyr/include/vreteno/regs"
mkdir -p "$dir"
maps=$("$regmap" list | cut -d: -f1)
for h in "$dir"/*.h; do
  [ -e "$h" ] || continue
  m=$(basename "$h" .h)
  printf '%s\n' $maps | grep -qx "$m" || rm -v "$h"
done
for m in $maps; do
  "$regmap" "$m" c > "$dir/$m.h"
  echo "wrote $dir/$m.h"
done
