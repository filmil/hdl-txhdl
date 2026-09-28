#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# A routed bitstream leaves the flash's program area alone (issue 312):
#   fits_test.sh BITFIT OFFSET FILE...
# FILE is every output of a place and route target; the `.bit` among
# them is the one read.
set -euo pipefail
bitfit="$1"
offset="$2"
shift 2
bits=()
for f in "$@"; do
	[[ "$f" == *.bit ]] && bits+=("$f")
done
if [[ ${#bits[@]} -eq 0 ]]; then
	echo "no .bit among: $*" >&2
	exit 2
fi
exec "$bitfit" -offset="$offset" "${bits[@]}"
