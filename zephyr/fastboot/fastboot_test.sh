#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Stock `fastboot`, from Android's platform tools, against the device
# side this repository runs on the board (issue 143). No client is
# written here: the point of choosing fastboot was that the host tool
# already exists, so the tool is the test.
#
#   fastboot_test.sh HARNESS FASTBOOT
#
# The harness serves the same core the Zephyr server runs, on a free
# port of 127.0.0.1. The tool reads `max-download-size`, then `fastboot
# boot` wraps an image in a boot image, downloads it and boots it, and
# the program the harness finds in what was staged must be the image,
# byte for byte. The image is 100 003 bytes, so it is not a multiple
# of a word or of a page and it takes many TCP segments.
set -euo pipefail

harness="$1"
fastboot="$2"
work="${TEST_TMPDIR:-$(mktemp -d)}"

head -c 100003 /dev/urandom >"$work/image.bin"
rm -f "$work/port" "$work/out.bin"

"$harness" "$work/port" "$work/out.bin" &
pid=$!
trap 'kill "$pid" 2>/dev/null || true' EXIT

for _ in $(seq 100); do
	[[ -s "$work/port" ]] && break
	sleep 0.05
done
port="$(cat "$work/port")"
target="tcp:127.0.0.1:$port"

size="$("$fastboot" -s "$target" getvar max-download-size 2>&1 |
	sed -n 's/^max-download-size: //p')"
if [[ "$size" != "0x01000000" ]]; then
	echo "max-download-size: expected 0x01000000, got '$size'" >&2
	exit 1
fi

"$fastboot" -s "$target" boot "$work/image.bin"

wait "$pid"
trap - EXIT

if ! cmp "$work/image.bin" "$work/out.bin"; then
	echo "the program staged is not the image sent" >&2
	exit 1
fi
echo "stock fastboot booted the image, byte for byte"
