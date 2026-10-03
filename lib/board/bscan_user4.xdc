# SPDX-License-Identifier: Apache-2.0
# The cable's clock out of the USER4 scan chain (issue 154). OpenOCD's
# adapter runs it at a few megahertz; 30 MHz bounds it. It is a clock
# of its own, crossing to the board's only through ChanCdc, so it is
# asynchronous to every other. The instance is named, because Vivado's
# debug hub has a BSCANE2 of its own on another chain.
create_clock -name tck -period 33.333 [get_pins -hierarchical -filter {NAME =~ *dbg_bscan/bscan/TCK}]
set_clock_groups -asynchronous -group [get_clocks tck]
