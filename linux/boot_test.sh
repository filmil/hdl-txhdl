#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# Linux on the machine model, to its shell (issue 1016, M9 of #279).
#
# Boots the image //linux:model_boot packs, the tree's own OpenSBI,
# kernel and initramfs with mem=64M, on //cpu/vreteno:machine, and
# passes when the console shows /init's marker and BusyBox's prompt.
# About 200 million instructions, a few minutes at the model's speed,
# so it is manual and the nightly linux workflow runs it.
set -o errexit -o nounset -o pipefail

machine="$1"
image="$2"
out="${TEST_TMPDIR:-$(mktemp -d)}/console.txt"

"$machine" --image "$image" --at 0x40000000 --steps 250000000 \
    < /dev/null > "$out" 2> "$out.err" || true
cat "$out"
tail -2 "$out.err"

fail=0
grep -q "txhdl: userspace is up" "$out" ||
    { echo "FAIL: /init never said userspace is up" >&2; fail=1; }
grep -q "^~ # " "$out" ||
    { echo "FAIL: no shell prompt" >&2; fail=1; }
[ "$fail" -eq 0 ] && echo "PASS: userspace is up, and the shell prompts"
exit "$fail"
