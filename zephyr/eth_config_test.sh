#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# The Ethernet driver is in the image, and not merely in the tree.
#
# `ETH_VRETENO` depends on `NETWORKING`, so a build without a stack
# leaves the driver uncompiled and every fault in it unfound. This
# checks the build that has one.
#
# That distinction is the whole reason this file exists.
# `uart_vreteno.c` was written, committed, and checked by a test that
# reads it as text, and no compiler saw it for as long as the module
# was absent from the build; it kept an implicit declaration of
# `sys_read32` the entire time. A driver in the tree is not a driver
# that builds.
set -eu

config=$1
fail=0

want() {
	if grep -qx "$1" "$config"; then
		echo "ok      $1"
	else
		echo "MISSING $1"
		fail=1
	fi
}

# The driver was compiled, and the stack counts it. The second is what
# `SERIAL_HAS_DRIVER` is for the console: without it the subsystem
# never looks for the interface.
want "CONFIG_ETH_VRETENO=y"
want "CONFIG_ETH_DRIVER=y"

# And the interface is an Ethernet one.
want "CONFIG_NET_L2_ETHERNET=y"

# The stack's random numbers come from the entropy source and not
# from the test generator, which is a counter (issue 458). The first
# line is the driver, the second what makes the subsystem look for a
# device, the third the generator it then picks.
want "CONFIG_ENTROPY_VRETENO=y"
want "CONFIG_ENTROPY_HAS_DRIVER=y"
want "CONFIG_ENTROPY_DEVICE_RANDOM_GENERATOR=y"
if grep -qx "CONFIG_TEST_RANDOM_GENERATOR=y" "$config"; then
	echo "PRESENT CONFIG_TEST_RANDOM_GENERATOR=y, a counter is not random"
	fail=1
else
	echo "ok      no test random generator"
fi

if [ "$fail" -ne 0 ]; then
	echo
	echo "The Ethernet driver is not in this image."
	echo "See $config"
	exit 1
fi
