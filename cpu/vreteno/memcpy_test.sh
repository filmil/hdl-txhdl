#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# The HAL's memcpy (#1825) on the machine: memcpy_check.rs copies every
# length from 0 to 100 bytes, from and to each offset in a word, and
# checks each against a byte loop, the bytes around it and the pointer
# returned. 16 pairs of offsets by 101 lengths is 1616 cases.
#
#   memcpy_test.sh <machine> <memcpy_check_ram_bin.bin>
set -eu
machine="$1"
image="$2"
out=$(mktemp)
"$machine" --image "$PWD/$image" --at 0x40000000 --steps 100000000 \
  >"$out" 2>&1 || true
cat "$out"
grep -q "^memcpy ok 1616\$" "$out" || { echo "FAIL: no 'memcpy ok 1616'" >&2; exit 1; }
echo "PASS: memcpy copies as a byte loop does"
