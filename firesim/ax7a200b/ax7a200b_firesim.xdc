# SPDX-License-Identifier: Apache-2.0
# FireSim's GCD example on the AX7A200B, with Ethernet as the host link
# (issue 1931). The board's pins and clock are //eth's echo_xdc.
#
# GCD's clock bridge feeds the simulator's 50 MHz clock into three
# BUFGCEs, one for the bridge and one for each of GCD's two copies, and
# a BUFG can be adjacent to only one other, so the placer rejects the
# cascade (rule_cascaded_bufg). FireSim's Nitefury build of GCD needed
# the same override; its Rocket designs, with one target clock, don't.
set_property CLOCK_DEDICATED_ROUTE FALSE [get_nets clk50]

# The answers' bytes cross from the simulator's 50 MHz to the PHY's
# 125 MHz in chan_cdc, whose gray-coded pointers are the only paths
# between them. Both clocks come from one MMCM, so without this Vivado
# times those paths as synchronous. The receive clock is already apart
# from both (//eth's eth_pins.xdc).
set_clock_groups -asynchronous \
  -group [get_clocks -of_objects [get_pins mmcm/CLKOUT1]] \
  -group [get_clocks -of_objects [get_pins mmcm/CLKOUT0]]
