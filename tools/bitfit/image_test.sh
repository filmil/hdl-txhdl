#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# A flash image holds a program at the programs' offset (issue 312):
#   image_test.sh BITFIT OFFSET PROGRAM FILE...
# FILE is every output of a flash target; the `.mcs` among them is the
# one read.
set -euo pipefail
bitfit="$1"
offset="$2"
program="$3"
shift 3
images=()
for f in "$@"; do
	[[ "$f" == *.mcs ]] && images+=("$f")
done
if [[ ${#images[@]} -ne 1 ]]; then
	echo "want one .mcs among: $*" >&2
	exit 2
fi
exec "$bitfit" -offset="$offset" -image="${images[0]}" -program="$program"
