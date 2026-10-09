#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# The conformance suite on the host (#999): runs it, prints its report,
# and holds its failures to the known ones, each with its issue. A new
# failure fails the test, and so does a known one that passes, so that
# the list stays the list of what is wrong.
set -euo pipefail

run="$1"
known="$2"

report="$("$run")" || {
  echo "$report"
  echo "the suite did not finish"
  exit 1
}
echo "$report"

failed="$(echo "$report" | sed -n 's/^FAIL \([^:]*\):.*/\1/p' | sort)"
expected="$(grep -v '^#' "$known" | awk 'NF { print $1 }' | sort)"
new="$(comm -23 <(echo "$failed") <(echo "$expected") | sed '/^$/d')"
fixed="$(comm -13 <(echo "$failed") <(echo "$expected") | sed '/^$/d')"
status=0
if [ -n "$new" ]; then
  echo "failures not in $known:"
  echo "$new"
  status=1
fi
if [ -n "$fixed" ]; then
  echo "known failures that pass now; take them off $known:"
  echo "$fixed"
  status=1
fi
echo "$report" | grep -q '^DONE ' || {
  echo "no count at the end"
  status=1
}
exit $status
