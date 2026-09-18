# SPDX-License-Identifier: Apache-2.0
# The pins the board's own design uses, less the memory: the 200 MHz
# differential clock, the reset button, the four LEDs and the serial
# port. The I/O standards are the ones cpu/vreteno/board/ax7a200.xdc
# states, so that a run here says something about a run there.
create_clock -add -name sys_clk_p -period 5.0 -waveform {0 2.5} [get_ports {sys_clk_p}]
set_property -dict { PACKAGE_PIN R4 IOSTANDARD DIFF_SSTL15 } [get_ports { sys_clk_p }]
set_property -dict { PACKAGE_PIN T4 IOSTANDARD DIFF_SSTL15 } [get_ports { sys_clk_n }]
set_property -dict { PACKAGE_PIN F15 IOSTANDARD LVCMOS33 } [get_ports { reset_n }]
set_property -dict { PACKAGE_PIN L13 IOSTANDARD LVCMOS33 } [get_ports { led1 }]
set_property -dict { PACKAGE_PIN M13 IOSTANDARD LVCMOS33 } [get_ports { led2 }]
set_property -dict { PACKAGE_PIN K14 IOSTANDARD LVCMOS33 } [get_ports { led3 }]
set_property -dict { PACKAGE_PIN K13 IOSTANDARD LVCMOS33 } [get_ports { led4 }]
set_property -dict { PACKAGE_PIN L15 IOSTANDARD LVCMOS33 } [get_ports { uart_tx }]
set_property CONFIG_VOLTAGE 3.3 [current_design]
set_property CFGBVS VCCO [current_design]
