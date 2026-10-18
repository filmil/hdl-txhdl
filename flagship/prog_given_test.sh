#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# flagship_prog_given refuses what it should before it reaches a board
# (#1784, #1803): a file whose sum is not the one given, a file that is
# no bitstream, and a missing file are each refused before the script
# starts hw_server, so no run here touches a board.
#
# The script is run as `bazel run` lays it out: beside its runfiles tree
# and a manifest of every runfile's real place, which the runfiles
# library prefers. Run through the runfiles directory alone, as a
# test's would be, it stops at its first lookup of fshlib, which fails
# under its `set -e` before it tries the name that is there
# (rules_vivado 4.1.0; reported upstream).
set -uo pipefail

dir="$(realpath "${TEST_TMPDIR:-$(mktemp -d)}")"
tree="${RUNFILES_DIR:-$TEST_SRCDIR}"
prog="$dir/run/prog"
mkdir -p "$dir/run"
cp -- "$1" "$prog"
ln -s "$tree" "$prog.runfiles"
(cd "$tree" && find -L . -type f | sed 's|^\./||' | while read -r f; do
  printf '%s %s\n' "$f" "$(readlink -f "$f")"
done) >"$prog.runfiles_manifest"
bit="$dir/good.bit"
printf '\x00\x09\x0f\xf0\x0f\xf0\x0f\xf0\x0f\xf0\x00\x00\x01rest' >"$bit"
sum="$(sha256sum "$bit" | cut -d' ' -f1)"
junk="$dir/junk.bit"
printf 'not a bitstream' >"$junk"
jsum="$(sha256sum "$junk" | cut -d' ' -f1)"
zero="$(printf '0%.0s' $(seq 64))"
fail=0

refused() { # refused <text> <args...>
  local text="$1"
  shift
  local out status
  out="$(env -u RUNFILES_DIR -u RUNFILES_MANIFEST_FILE -u TEST_SRCDIR \
    "$prog" --hostport localhost:1 "$@" 2>&1)"
  status=$?
  if [[ "$status" == 0 || "$out" != *"$text"* ]]; then
    echo "FAIL: $* gave $status, wanted a refusal naming \"$text\":"
    echo "$out"
    fail=1
  fi
  if [[ "$out" == *"hw_server"* && "$out" == *"Running programmer"* ]]; then
    echo "FAIL: $* started hw_server before refusing"
    fail=1
  fi
}

refused "has SHA-256 $sum, not $zero" --bitfile="$bit" --sha256="$zero"
refused "not a bitstream" --bitfile="$junk" --sha256="$jsum"
refused "$dir/none.bit" --bitfile="$dir/none.bit" --sha256="$sum"
exit "$fail"
