#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# The board's OpenOCD configuration is read by the pinned OpenOCD, up to
# the point where it would open the cable: every command in it exists and
# takes what it is given, and the tunnel is selected (#895). Opening the
# cable is the board's part, in docs/board-checks.md section 4.
set -euo pipefail
log=$("$OPENOCD" -f "$CFG" -c shutdown 2>&1) || {
  echo "$log"
  echo "FAIL: OpenOCD refused $CFG"
  exit 1
}
grep -q "Bscan Tunnel Selected" <<<"$log" || {
  echo "$log"
  echo "FAIL: the BSCAN tunnel was not selected"
  exit 1
}
echo "ok: $CFG"
