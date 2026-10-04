#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# The image is this machine's, and not a stock Zephyr that happened to
# link.
#
# An ELF existing proves very little here, which this port demonstrated
# four separate times before it ever printed: it built with the module
# absent, with no clock, with a console that was not a console, and
# with a driver no compiler had seen. Every one of those produced a
# file of the right shape.
#
# So what is checked is the configuration the build generated, where
# each of those faults would show.
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

# The console is SiFive's driver on the port (issue 1011): the device
# tree's `sifive,uart0` node turned it on, and the board turned its
# first port on.
want "CONFIG_UART_SIFIVE=y"
want "CONFIG_UART_SIFIVE_PORT_0=y"

# The driver announced itself, so the console is a console. Missing
# this makes `CONFIG_UART_CONSOLE` invisible rather than off, and the
# board's own defconfig asking for it is dropped without a word.
want "CONFIG_SERIAL_HAS_DRIVER=y"
want "CONFIG_UART_CONSOLE=y"

# The SoC and the board are this machine's, which also says the module
# was reached at all: without `-DZEPHYR_MODULES` these go missing, and
# nothing else complains, the image having none of this repository in it.
want "CONFIG_SOC_VRETENO=y"
want 'CONFIG_BOARD="ax7a200b"'

# The clock. The timer's node claimed a binding this does not select
# on once, and the image built without a clock and said nothing.
want "CONFIG_RISCV_MACHINE_TIMER=y"

# The core is RV32IMC, so the atomics are the C ones. An image that
# claimed the A extension would trap on its first atomic.
want "CONFIG_ATOMIC_OPERATIONS_C=y"

if [ "$fail" -ne 0 ]; then
	echo
	echo "The generated configuration is not this machine's."
	echo "See $config"
	exit 1
fi
