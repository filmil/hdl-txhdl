#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Write a given flagship bitstream into the FPGA, once its sum is the
# one given (#1784). A place and route takes one to two hours and is
# not bit for bit the same twice, so the session at the board programs
# the bitstream another session built and checked, rather than building
# its own:
#
#   bazel run //flagship:flagship_prog_given -- \
#       --bit=/data/tmp/ai/shared/flagship-1779-78496c91/flagship_pnr.bit \
#       --sha256="$sum" \
#       --hostport localhost:3122 --device '*/xilinx_tcf/Digilent/*'
#
# where $sum is the 64 hex digits the session that built it reported.
#
# It refuses a file whose sha256 is not the one given, or that does not
# start with a bitstream's 13-byte header, and says the sum it
# programmed, so a board log names the bitstream that ran. With
# --dry-run it checks and says, and programs nothing. Every other
# argument goes to the programming script, as flagship_prog's do.
set -euo pipefail

# --- begin runfiles.bash initialization v3 ---
f=bazel_tools/tools/bash/runfiles/runfiles.bash
# shellcheck disable=SC1090
source "${RUNFILES_DIR:-/dev/null}/$f" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "${RUNFILES_MANIFEST_FILE:-/dev/null}" | cut -f2- -d' ')" 2>/dev/null || \
  source "$0.runfiles/$f" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "$0.runfiles_manifest" | cut -f2- -d' ')" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "$0.exe.runfiles_manifest" | cut -f2- -d' ')" 2>/dev/null || \
  { echo>&2 "ERROR: cannot find $f"; exit 1; }; f=
# --- end runfiles.bash initialization v3 ---

bit=""
sum=""
dry=""
rest=()
for a in "$@"; do
  case "$a" in
    --bit=*) bit="${a#*=}" ;;
    --sha256=*) sum="${a#*=}" ;;
    --dry-run) dry=1 ;;
    *) rest+=("$a") ;;
  esac
done
if [[ -z "$bit" || -z "$sum" ]]; then
  echo "usage: flagship_prog_given --bit=PATH --sha256=HEX" \
    "[--dry-run] [args]" >&2
  exit 2
fi
if [[ ! -f "$bit" ]]; then
  echo "refused: no such file: $bit" >&2
  exit 1
fi
bit="$(realpath -- "$bit")"
got="$(sha256sum -- "$bit" | cut -d' ' -f1)"
if [[ "$got" != "${sum,,}" ]]; then
  echo "refused: $bit has sha256 $got, not $sum" >&2
  exit 1
fi
# The header the programming script checks as well.
if [[ "$(head -c 13 -- "$bit" | od -An -tx1 | tr -d ' \n')" \
  != "00090ff00ff00ff00ff0000001" ]]; then
  echo "refused: not a bitstream: $bit" >&2
  exit 1
fi
echo "programming $bit, sha256 $got"
if [[ -n "$dry" ]]; then
  echo "dry run: nothing programmed"
  exit 0
fi

# The generated script names its bitstream and its tools by paths in
# the main repository's runfiles, from where it runs. So it runs from
# a copy of that tree, made of links, with the given file in the
# bitstream's place.
base="$(rlocation txhdl/flagship/flagship_prog_given_base.sh)"
main="$(dirname "$(dirname "$base")")"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
for e in "$main"/*; do
  ln -s "$e" "$tmp/"
done
rm "$tmp/flagship"
mkdir "$tmp/flagship"
for e in "$main"/flagship/*; do
  ln -s "$e" "$tmp/flagship/"
done
rm "$tmp/flagship/given.bit"
cp -- "$bit" "$tmp/flagship/given.bit"
export RUNFILES_DIR
RUNFILES_DIR="$(dirname "$main")"
cd "$tmp"
"$base" "${rest[@]}"
echo "programmed $bit, sha256 $got"
