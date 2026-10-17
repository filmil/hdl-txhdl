#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# The application outside the port's module (issue 1772) was built for
# the port's board and with the TCP stack its prj.conf asked for.
set -eu
config=$1
fail=0
for want in \
	'CONFIG_BOARD="ax7a200b"' \
	'CONFIG_NET_TCP=y' \
	'CONFIG_NET_SOCKETS=y' \
	'CONFIG_ETH_VRETENO=y'; do
	if grep -qx "$want" "$config"; then
		echo "ok      $want"
	else
		echo "MISSING $want"
		fail=1
	fi
done
exit $fail
