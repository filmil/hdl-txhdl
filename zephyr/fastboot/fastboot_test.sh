#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Stock `fastboot`, from Android's platform tools, against the device
# side this repository runs on the board (issue 143). No client is
# written here: the point of choosing fastboot was that the host tool
# already exists, so the tool is the test.
#
#   fastboot_test.sh HARNESS FASTBOOT PROGRAM PADDED
#
# The harness serves the same core the Zephyr server runs, on a free
# port of 127.0.0.1. The tool reads `max-download-size`, then `fastboot
# boot` wraps an image in a boot image, downloads it and boots it, and
# the program the harness finds in what was staged must be the image,
# byte for byte. The image is 100 003 bytes, so it is not a multiple
# of a word or of a page and it takes many TCP segments.
#
# Then a program as small as `hello_ram_bin`, PROGRAM: the tool refuses
# it, since it reads a boot image header's worth of a file before it
# decides whether the file is one (issue 799), and boots PADDED, the
# same program padded by `vreteno_fastboot`, byte for byte, with the
# program unchanged at its start.
set -euo pipefail

harness="$1"
fastboot="$2"
program="$3"
padded="$4"
work="${TEST_TMPDIR:-$(mktemp -d)}"

# Starts the harness, which serves one client and exits, and leaves
# its port in `target`.
serve() {
	rm -f "$work/port" "$work/out.bin"
	"$harness" "$work/port" "$work/out.bin" &
	pid=$!
	trap 'kill "$pid" 2>/dev/null || true' EXIT
	for _ in $(seq 100); do
		[[ -s "$work/port" ]] && break
		sleep 0.05
	done
	target="tcp:127.0.0.1:$(cat "$work/port")"
}

# Boots an image and checks the harness staged it byte for byte.
boot() {
	serve
	"$fastboot" -s "$target" boot "$1"
	wait "$pid"
	trap - EXIT
	if ! cmp "$1" "$work/out.bin"; then
		echo "the program staged is not the image sent: $1" >&2
		exit 1
	fi
}

head -c 100003 /dev/urandom >"$work/image.bin"

serve
size="$("$fastboot" -s "$target" getvar max-download-size 2>&1 |
	sed -n 's/^max-download-size: //p')"
if [[ "$size" != "0x01000000" ]]; then
	echo "max-download-size: expected 0x01000000, got '$size'" >&2
	exit 1
fi
kill "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
trap - EXIT

boot "$work/image.bin"
echo "stock fastboot booted the image, byte for byte"

# The program alone is refused by the tool before anything is sent,
# though it waits for a device first, so one is served. If this starts
# to succeed, a newer tool has dropped the limit and `vreteno_fastboot`
# can go.
serve
refused=0
timeout 60 "$fastboot" -s "$target" boot "$program" 2>"$work/err" || refused=1
kill "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
trap - EXIT
if [[ $refused == 0 ]]; then
	echo "fastboot booted $(wc -c <"$program") bytes unpadded" >&2
	exit 1
fi
if ! grep -q "too short" "$work/err"; then
	echo "fastboot refused the program for another reason:" >&2
	cat "$work/err" >&2
	exit 1
fi
echo "stock fastboot refuses $(wc -c <"$program") bytes: too short"

if ! cmp -n "$(wc -c <"$program")" "$program" "$padded"; then
	echo "the padded image does not start with the program" >&2
	exit 1
fi
if [[ -n "$(tail -c +"$(($(wc -c <"$program") + 1))" "$padded" | tr -d '\0')" ]]; then
	echo "the padding is not zeros" >&2
	exit 1
fi
boot "$padded"
echo "stock fastboot booted the padded program, $(wc -c <"$padded") bytes"
