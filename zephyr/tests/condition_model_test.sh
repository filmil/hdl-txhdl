#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# A source shaped like the board's, through the driver's conditioner
# (issue 780): trngstat fails the extractor's words as they come, as it
# fails the board's, and passes them conditioned.
#   condition_model_test.sh MODEL TRNGSTAT
set -euo pipefail
model="$1"
trngstat="$2"
dir="${TEST_TMPDIR:-$(mktemp -d)}"
"$model" "$dir/unconditioned.log" "$dir/conditioned.log"

if "$trngstat" "$dir/unconditioned.log" > "$dir/unconditioned.txt"; then
	cat "$dir/unconditioned.txt"
	echo "FAIL: trngstat passed the unconditioned words, so the model does not bite"
	exit 1
fi
grep -E "correlation at lag|bias|min-entropy" "$dir/unconditioned.txt" | head -5
echo "ok: the unconditioned words fail, as the board's do"

if ! "$trngstat" "$dir/conditioned.log" > "$dir/conditioned.txt"; then
	cat "$dir/conditioned.txt"
	echo "FAIL: trngstat failed the conditioned words"
	exit 1
fi
echo "ok: the conditioned words pass"
