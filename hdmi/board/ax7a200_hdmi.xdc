# SPDX-License-Identifier: Apache-2.0
# The HDMI demonstration on the Alinx AX7A200B (xc7a200tfbg484-2).
#
# The clock and the LEDs are the ones cpu/vreteno/board/ax7a200.xdc
# names. The SiI9134's pins are from the table of the AX7A200B manual's
# section on the HDMI output. The manual states neither the I/O standard
# of those pins nor which of the chip's data inputs are red, green and
# blue; LVCMOS33, as for the board's other user pins, and red on the top
# eight bits are to be confirmed on the board.

# The board's 200 MHz clock, and the LEDs.
create_clock -add -name sys_clk_p -period 5.0 -waveform {0 2.5} \
  [get_ports {sys_clk_p}]
set_property PACKAGE_PIN R4 [get_ports {sys_clk_p}]
set_property PACKAGE_PIN T4 [get_ports {sys_clk_n}]
set_property IOSTANDARD DIFF_SSTL15 [get_ports {sys_clk_*}]
set_property PACKAGE_PIN L13 [get_ports {led1}]
set_property PACKAGE_PIN M13 [get_ports {led2}]
set_property PACKAGE_PIN K14 [get_ports {led3}]
set_property IOSTANDARD LVCMOS33 [get_ports {led*}]

set_property CONFIG_VOLTAGE 3.3 [current_design]
set_property CFGBVS VCCO [current_design]

# The board has four LEDs and this design names three, so K13 is an
# unused pin, and a 7 series bitstream pulls unused pins down by
# default, which leaves that LED glowing faintly. See issue #232.
set_property BITSTREAM.CONFIG.UNUSEDPIN PULLNONE [current_design]
