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
# Each reader's failing case, on a log made for it (agy on PR 1856).
bad="$(mktemp)"
trap 'rm -f "$now" "$with_header" "$empty" "$bad"' EXIT
# OpenSBI 15 s after booting, #1457's gap, and no /init: both fail.
printf '00:00:01.000Z fastboot: booting 1 bytes at 0x40000000\n00:00:16.000Z OpenSBI v1.6\n' >"$bad"
[[ "$(read_linux linux "$bad" | cut -f5 | tr '\n' ' ')" == "fail fail " ]]
# A demo that never reached its end, and one that dropped frames.
printf '00:00:01.000Z demo ico 5 tx_dropped 3\n' >"$bad"
[[ "$(read_demo d "$bad" "" | cut -f5 | tr '\n' ' ')" == "fail fail " ]]
# The PHY with the link down and no 1000BASE-T partner.
printf '00:00:01.000Z r00 1140 7969 937c 4032\n00:00:01.000Z r08 0000 0200 0000 0000\n' >"$bad"
[[ "$(read_phy "$bad" | cut -f5 | tr '\n' ' ')" == "fail fail " ]]
# The dump's read of r1 says down, as after programming, and the second
# read says up: the second is the link (#1897). And the other way.
printf '00:00:01.000Z r00 1140 7969 937c 4032\n00:00:01.000Z r08 0000 0200 7800 0000\n00:00:01.000Z r01 again 796d\n' >"$bad"
[[ "$(read_phy "$bad" | head -1 | cut -f3,5)" == "up	pass" ]]
printf '00:00:01.000Z r00 1140 796d 937c 4032\n00:00:01.000Z r08 0000 0200 7800 0000\n00:00:01.000Z r01 again 7969\n' >"$bad"
[[ "$(read_phy "$bad" | head -1 | cut -f3,5)" == "down	fail" ]]
# A receive with nothing sent is not a pass.
printf '00:00:01.000Z rx dma 60 got 0 of 0 lost 0 other 0 ticks 0 cycles/frame 0\n' >"$bad"
[[ "$(read_ethperf "$bad" | cut -f5)" == fail ]]
# A paced burst passes at 99% or more of the frames sent (#1955).
printf '00:00:01.000Z rx dma 60 got 990 of 1000 lost 10 other 0 ticks 1 cycles/frame 1\n' >"$bad"
[[ "$(read_ethperf "$bad" | cut -f5)" == pass ]]
printf '00:00:01.000Z rx dma 60 got 989 of 1000 lost 11 other 0 ticks 1 cycles/frame 1\n' >"$bad"
[[ "$(read_ethperf "$bad" | cut -f5)" == fail ]]
# More got than sent is not a pass either.
printf '00:00:01.000Z rx dma 60 got 1001 of 1000 lost 0 other 0 ticks 1 cycles/frame 1\n' >"$bad"
[[ "$(read_ethperf "$bad" | cut -f5)" == fail ]]
# Each burst's gap: #1038's 40 us for the 60-byte frames, 60 for 1514.
[[ "$(rx_gap_us 60)" == 40 && "$(rx_gap_us 1514)" == 60 ]]
# Listening with no load before it is not a time.
printf '00:00:01.000Z fastboot: listening on port 5554\n' >"$bad"
[[ "$(read_fastboot "$bad" yes | head -1 | cut -f5)" == fail ]]
# A binary run bare sees none of this script's runfiles variables, so a
# wrapper finds its own files and not the nightly's (#1898).
[[ "$(RUNFILES_DIR=/x RUNFILES_MANIFEST_FILE=/y JAVA_RUNFILES=/z bare bash -c \
  'echo "${RUNFILES_DIR-unset} ${RUNFILES_MANIFEST_FILE-unset} ${JAVA_RUNFILES-unset}"')" \
  == "unset unset unset" ]]
echo "nightly readers: pass"
