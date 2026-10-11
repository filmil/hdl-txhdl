#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# The hot linker script (#1679) keeps a program's start first, where the
# loader enters it, then the hot list, then the rest of the text.
set -euo pipefail

ld=$1
line() { grep -n -F -- "$1" "$ld" | head -1 | cut -d: -f1 || true; }
init=$(line '*(.text.init)')
first=$(line '*(.text._ZN')
rest=$(line '*(.text .text.*)')
hot=$(grep -c -F -- '*(.text.' "$ld")
if [ -z "$init" ] || [ -z "$first" ] || [ -z "$rest" ]; then
  echo "FAIL: $ld lacks the start, the hot list or the rest" >&2
  exit 1
fi
if ! [ "$init" -lt "$first" ] || ! [ "$first" -lt "$rest" ]; then
  echo "FAIL: start at line $init, hot list at $first, rest at $rest" >&2
  exit 1
fi
if [ "$hot" -lt 20 ]; then
  echo "FAIL: only $hot text patterns" >&2
  exit 1
fi
echo "PASS: start at line $init, $((hot - 2)) hot patterns from line $first, the rest at $rest"
