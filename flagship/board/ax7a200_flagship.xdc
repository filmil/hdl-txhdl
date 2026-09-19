# SPDX-License-Identifier: Apache-2.0
# The flagship on the Alinx AX7A200B (xc7a200tfbg484-2): what this
# design needs that the files it shares do not already say.
#
# The board's clock, its reset, its LEDs, its serial port and its DDR3
# memory come from cpu/vreteno/board/ax7a200.xdc and
# cpu/vreteno/board/ax7a200_ddr3.xdc; the PHY's pins from
# eth/board/eth_pins.xdc; the encoder's from hdmi/board/hdmi_pins.xdc.
# Those four files are the one copy of each pin in the tree, and this
# design reads them rather than restating them, so a pin corrected for
# one design is corrected for the flagship in the same change.
#
# What is left is the part no single subsystem owns: the three clock
# generators hang off one input, and nothing crosses between their
# domains except through the asynchronous FIFOs, so the three are told
# apart for timing.

# The pixel clock, the Ethernet's transmit clock and the memory's
# clocks are generated from one input, so Vivado relates all of them
# and would time paths that do not exist. The Ethernet's receive clock
# is already grouped apart in eth/board/eth_pins.xdc, which names the
# generated clocks of sys_clk_p as one group; this splits that group by
# generator. The core's AXI-Lite reaches the video peripheral, and the
# remote peripheral's frames reach the transmit half, only through
# chan_cdc, whose two sides are asynchronous by construction.
set core_clk [get_clocks -include_generated_clocks \
  -of_objects [get_pins pll/CLKOUT0]]
set pixel_clk [get_clocks -include_generated_clocks \
  -of_objects [get_pins vid_mmcm/CLKOUT0]]
set eth_tx_clk [get_clocks -include_generated_clocks \
  -of_objects [get_pins eth_mmcm/CLKOUT0]]
set_clock_groups -asynchronous \
  -group $core_clk -group $pixel_clk -group $eth_tx_clk

# Unused pins float rather than being pulled down, so nothing the
# design does not name is driven weakly on the board. See issue #232.
set_property BITSTREAM.CONFIG.UNUSEDPIN PULLNONE [current_design]
