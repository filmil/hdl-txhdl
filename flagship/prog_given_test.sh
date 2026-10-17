#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# flagship_prog_given refuses what it should and programs nothing in a
# dry run (#1784): a wrong sum, a file that is no bitstream, and missing
# arguments are refused; a file with the bitstream's header and the
# right sum passes, and the run names its sum.
set -uo pipefail

prog="$1"
dir="${TEST_TMPDIR:-$(mktemp -d)}"
bit="$dir/good.bit"
printf '\x00\x09\x0f\xf0\x0f\xf0\x0f\xf0\x0f\xf0\x00\x00\x01rest' >"$bit"
sum="$(sha256sum "$bit" | cut -d' ' -f1)"
junk="$dir/junk.bit"
printf 'not a bitstream' >"$junk"
jsum="$(sha256sum "$junk" | cut -d' ' -f1)"
fail=0

expect() { # expect <status> <text> <args...>
  local want="$1" text="$2"
  shift 2
  local out status
  out="$("$prog" "$@" 2>&1)"
  status=$?
  if [[ "$status" != "$want" || "$out" != *"$text"* ]]; then
    echo "FAIL: $* gave $status, wanted $want and \"$text\":"
    echo "$out"
    fail=1
  fi
}

zero="$(printf '0%.0s' $(seq 64))"
expect 1 "refused: $bit has sha256 $sum, not $zero" \
  --bit="$bit" --sha256="$zero" --dry-run
expect 1 "refused: not a bitstream" --bit="$junk" --sha256="$jsum" --dry-run
expect 1 "refused: no such file" --bit="$dir/none.bit" --sha256="$sum"
expect 2 "usage" --bit="$bit"
expect 0 "programming $bit, sha256 $sum" \
  --bit="$bit" --sha256="${sum^^}" --dry-run
expect 0 "dry run: nothing programmed" \
  --bit="$bit" --sha256="$sum" --dry-run
exit "$fail"
