#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# flagship_prog_given refuses what it should before it reaches a board
# (#1784, #1803): a file whose sum is not the one given, a file that is
# no bitstream or is empty, and a missing file are each refused before
# the script starts hw_server, so no run here touches a board. A sum in
# capitals is the same sum, and a relative --bitfile is taken from where
# `bazel run` was started, not from where the script runs.
#
# The script runs through this test's runfiles, which rules_vivado
# 4.1.1 finds (#1821), in a directory of its own, where it copies the
# file it checks.
set -uo pipefail

dir="$(realpath "${TEST_TMPDIR:-$(mktemp -d)}")"
prog="$(realpath "$1")"
export RUNFILES_DIR="${RUNFILES_DIR:-$TEST_SRCDIR}"
mkdir -p "$dir/work"
bit="$dir/good.bit"
printf '\x00\x09\x0f\xf0\x0f\xf0\x0f\xf0\x0f\xf0\x00\x00\x01rest' >"$bit"
sum="$(sha256sum "$bit" | cut -d' ' -f1)"
junk="$dir/junk.bit"
printf 'not a bitstream' >"$junk"
jsum="$(sha256sum "$junk" | cut -d' ' -f1)"
empty="$dir/empty.bit"
: >"$empty"
esum="$(sha256sum "$empty" | cut -d' ' -f1)"
zero="$(printf '0%.0s' $(seq 64))"
fail=0

refused() { # refused <text> <args...>
  local text="$1"
  shift
  local out status
  out="$(cd "$dir/work" && "$prog" --hostport localhost:1 "$@" 2>&1)"
  status=$?
  if [[ "$status" == 0 || "$out" != *"$text"* ]]; then
    echo "FAIL: $* gave $status, wanted a refusal naming \"$text\":"
    echo "$out"
    fail=1
  fi
  # The script says this once the file has passed its checks, before it
  # starts hw_server.
  if [[ "$out" == *"Programming the --bitfile"* ]]; then
    echo "FAIL: $* passed the checks and went on to program"
    fail=1
  fi
}

refused "has SHA-256 $sum, not $zero" --bitfile="$bit" --sha256="$zero"
refused "not a bitstream" --bitfile="$junk" --sha256="$jsum"
refused "not a bitstream" --bitfile="$junk" --sha256="${jsum^^}"
refused "not a bitstream" --bitfile="$empty" --sha256="$esum"
refused "no such file: $dir/none.bit" \
  --bitfile="$dir/none.bit" --sha256="$sum"
BUILD_WORKING_DIRECTORY="$dir" \
  refused "has SHA-256 $sum, not $zero" --bitfile=good.bit --sha256="$zero"
exit "$fail"
