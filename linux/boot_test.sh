#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# Linux on the machine model, to its shell (issue 1016, M9 of #279).
#
# Boots the image //linux:model_boot packs, the tree's own OpenSBI,
# kernel and initramfs with mem=64M, on //cpu/vreteno:machine, and
# passes when the console shows /init's marker and BusyBox's prompt.
# About 200 million instructions, a few minutes at the model's speed,
# so it is manual and the nightly linux workflow runs it.
#
# Arguments past the image go to the machine: boot_loaded_test passes
# --as-loaded, the serial port as the loader leaves it on the board,
# with a request the interrupt controller holds (issue 1136).
set -o errexit -o nounset -o pipefail

machine="$1"
image="$2"
shift 2
out="${TEST_TMPDIR:-$(mktemp -d)}/console.txt"

"$machine" --image "$image" --at 0x40000000 --steps "${STEPS:-250000000}" "$@" \
    < /dev/null > "$out" 2> "$out.err" || true
cat "$out"
tail -2 "$out.err"

fail=0
# A test may ask for one more line, as boot_net_test asks for the
# network's (issue 1203), and for more instructions in STEPS, as it
# does for its pings a second apart (issue 1246).
if [ -n "${EXPECT:-}" ]; then
    grep -q "$EXPECT" "$out" ||
        { echo "FAIL: never said: $EXPECT" >&2; fail=1; }
fi
# And a second line, as boot_root_test asks for both the dynamic
# program's and the ssh server's (issue 1439).
# More lines, separated by `;`.
if [ -n "${EXPECT_ALSO:-}" ]; then
    IFS=';' read -ra also <<<"$EXPECT_ALSO"
    for line in "${also[@]}"; do
        grep -q "$line" "$out" ||
            { echo "FAIL: never said: $line" >&2; fail=1; }
    done
fi
grep -q "txhdl: userspace is up" "$out" ||
    { echo "FAIL: /init never said userspace is up" >&2; fail=1; }
grep -q "^~ # " "$out" ||
    { echo "FAIL: no shell prompt" >&2; fail=1; }
# The shell has the console as its controlling tty, so Ctrl-C reaches
# what it runs (issue 1249).
if grep -q "can't access tty" "$out"; then
    echo "FAIL: the shell has no controlling tty" >&2
    fail=1
fi
[ "$fail" -eq 0 ] && echo "PASS: userspace is up, and the shell prompts"
exit "$fail"
