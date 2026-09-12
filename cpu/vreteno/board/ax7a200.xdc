# SPDX-License-Identifier: Apache-2.0
# The Alinx AX7A200 (xc7a200tfbg484-2), as the a200t examples state it:
# a 200 MHz differential clock, an active-low reset button, four LEDs
# lit when driven low, 3.3 V configuration.
create_clock -add -name sys_clk_p -period 5.0 -waveform {0 2.5} [get_ports {sys_clk_p}]
set_property -dict { PACKAGE_PIN R4 IOSTANDARD DIFF_SSTL15 } [get_ports { sys_clk_p }]
set_property -dict { PACKAGE_PIN T4 IOSTANDARD DIFF_SSTL15 } [get_ports { sys_clk_n }]
set_property -dict { PACKAGE_PIN F15 IOSTANDARD LVCMOS33 } [get_ports { reset_n }]
set_property -dict { PACKAGE_PIN L13 IOSTANDARD LVCMOS33 } [get_ports { led1 }]
set_property -dict { PACKAGE_PIN M13 IOSTANDARD LVCMOS33 } [get_ports { led2 }]
set_property -dict { PACKAGE_PIN K14 IOSTANDARD LVCMOS33 } [get_ports { led3 }]
set_property -dict { PACKAGE_PIN K13 IOSTANDARD LVCMOS33 } [get_ports { led4 }]

# The serial port's lines, to and from the board's USB serial bridge;
# the pins are the ones filmil/a200t_examples names for uart1_txd and
# uart1_rxd.
set_property -dict { PACKAGE_PIN L15 IOSTANDARD LVCMOS33 } [get_ports { uart_tx }]
set_property -dict { PACKAGE_PIN L14 IOSTANDARD LVCMOS33 } [get_ports { uart_rx }]
set_property CONFIG_VOLTAGE 3.3 [current_design]
set_property CFGBVS VCCO [current_design]
# The flash is written over a 4-bit SPI bus.
set_property BITSTREAM.CONFIG.SPI_BUSWIDTH 4 [current_design]

# The core's data memory lanes stay in block RAM, as in vreteno.xdc.
set_property RAM_STYLE BLOCK [get_cells -hierarchical -regexp {.*dmem[0-3]_reg.*}]
