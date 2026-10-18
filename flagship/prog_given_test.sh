#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# flagship_prog_given refuses what it should and programs nothing in a
# dry run (#1784): a wrong sum, a file that is no bitstream, a file too
# short to be one, an empty file, the placeholder itself and missing
# arguments are refused; a file with the bitstream's header and the
# right sum passes, with its flags written either way and its path
# relative to where bazel was run, and the run names its sum after
# building what it would run.
set -uo pipefail

prog="$1"
placeholder="$2"
dir="$(realpath "${TEST_TMPDIR:-$(mktemp -d)}")"
bit="$dir/good.bit"
printf '\x00\x09\x0f\xf0\x0f\xf0\x0f\xf0\x0f\xf0\x00\x00\x01rest' >"$bit"
sum="$(sha256sum "$bit" | cut -d' ' -f1)"
junk="$dir/junk.bit"
printf 'not a bitstream' >"$junk"
jsum="$(sha256sum "$junk" | cut -d' ' -f1)"
short="$dir/short.bit"
printf '\x00\x09\x0f' >"$short"
ssum="$(sha256sum "$short" | cut -d' ' -f1)"
empty="$dir/empty.bit"
: >"$empty"
esum="$(sha256sum "$empty" | cut -d' ' -f1)"
psum="$(sha256sum "$placeholder" | cut -d' ' -f1)"
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
expect 1 "refused: not a bitstream" --bit="$short" --sha256="$ssum" --dry-run
expect 1 "refused: not a bitstream" --bit="$empty" --sha256="$esum" --dry-run
expect 1 "refused: not a bitstream" \
  --bit="$placeholder" --sha256="$psum" --dry-run
expect 1 "refused: no such file" --bit="$dir/none.bit" --sha256="$sum"
expect 2 "usage" --bit="$bit"
expect 0 "programming $bit, sha256 $sum" \
  --bit="$bit" --sha256="${sum^^}" --dry-run
expect 0 "dry run: nothing programmed" \
  --bit="$bit" --sha256="$sum" --dry-run
expect 0 "programming $bit, sha256 $sum" \
  --bit "$bit" --sha256 "$sum" --dry-run
BUILD_WORKING_DIRECTORY="$dir" expect 0 "programming $dir/good.bit" \
  --bit=good.bit --sha256="$sum" --dry-run
# As `bazel run` starts it (#1797): the binary beside its runfiles tree
# and its manifest, which the runfiles library prefers, so rlocation
# names each file's real place rather than its place in the tree.
run="$dir/run"
mkdir -p "$run"
cp -- "$prog" "$run/w"
tree="${RUNFILES_DIR:-$TEST_SRCDIR}"
ln -s "$tree" "$run/w.runfiles"
(cd "$tree" && find -L . -type f | sed 's|^\./||' | while read -r f; do
  printf '%s %s\n' "$f" "$(readlink -f "$f")"
done) >"$run/w.runfiles_manifest"
out="$(env -u RUNFILES_DIR -u RUNFILES_MANIFEST_FILE -u TEST_SRCDIR \
  "$run/w" --bit="$bit" --sha256="$sum" --dry-run 2>&1)"
if [[ $? != 0 || "$out" != *"dry run: nothing programmed"* ]]; then
  echo "FAIL: as bazel run starts it:"
  echo "$out"
  fail=1
fi
exit "$fail"
