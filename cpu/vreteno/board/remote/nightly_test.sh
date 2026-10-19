#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# The nightly's readers and its diff on logs a board session kept
# (issue 1854): each reader's rows, and the diff against an earlier
# night's results, as expected.tsv and expected_diff.tsv say.
#
#   nightly_test.sh LIB TESTDATA/linux.log
set -euo pipefail
lib="$1"
d="$(dirname "$2")"
# shellcheck source=nightly_lib.sh
source "$lib"
now="$(mktemp)"
trap 'rm -f "$now"' EXIT
{
  read_linux linux "$d/linux.log"
  read_demo ico-5-mipmapped "$d/ico5.log" "demo ico 5 done"
  read_phy "$d/phyregs.log"
  read_ping "$d/ping.log"
  read_ethperf "$d/ethperf.log"
  read_fastboot "$d/linux.log" yes
  read_costs "$d/costs.log"
} >"$now"
diff -u "$d/expected.tsv" "$now"
# The diff reads a header, as results.tsv has one.
with_header="$(mktemp)"
trap 'rm -f "$now" "$with_header"' EXIT
{ printf 'check\tmetric\tvalue\tunit\tstatus\n'; cat "$now"; } >"$with_header"
diff -u "$d/expected_diff.tsv" <(diff_results "$d/prev.tsv" "$with_header")
# A log with nothing to read says so as a fail, rather than nothing.
empty="$(mktemp)"
trap 'rm -f "$now" "$with_header" "$empty"' EXIT
[[ "$(read_phy "$empty" | cut -f5)" == fail ]]
[[ "$(read_ping "$empty" | cut -f5)" == fail ]]
[[ "$(read_ethperf "$empty" | cut -f5)" == fail ]]
[[ "$(read_costs "$empty" | cut -f5)" == fail ]]
[[ "$(read_linux linux "$empty" | cut -f5)" == fail ]]
echo "nightly readers: pass"
