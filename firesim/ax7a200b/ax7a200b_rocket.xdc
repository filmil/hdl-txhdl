# SPDX-License-Identifier: Apache-2.0
# FireSim's Rocket design on the AX7A200B (issue 1988). The board's
# pins and clock are //eth's echo_xdc; the DDR3's pins are the Vreteno
# board's.
#
# The frames' bytes cross between the simulator's 50 MHz and the PHY's
# 125 MHz in chan_cdc, whose gray-coded pointers are the only paths
# between them. Both clocks come from one MMCM, so without this Vivado
# times those paths as synchronous, as for GCD (ax7a200b_firesim.xdc).
set_clock_groups -asynchronous \
  -group [get_clocks -of_objects [get_pins mmcm/CLKOUT1]] \
  -group [get_clocks -of_objects [get_pins mmcm/CLKOUT0]]

# The memory's calibration and reset reach the simulator's reset through
# two flip-flops on the 50 MHz clock; the first of them is the crossing.
set_false_path -to [get_cells {sim_rst_sync_reg[0]}]

# The reset frame's request, held seventeen cycles of the simulator's
# clock, into the board's (issue 2007).
set_false_path -to [get_cells {soft_sync_reg[0]}]
