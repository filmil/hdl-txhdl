# SPDX-License-Identifier: Apache-2.0
# PCIe on the Alinx AX7A200B (xc7a200tfbg484-2). The lanes and the
# reference clock are the manual's, rev B, as issue 168 quotes it: the
# transceivers' pins take no I/O standard. The slot's PERST# is not in
# that table (issue 179), so the endpoint's reset is the board's reset
# key, as in ax7a200.xdc, and so are the four LEDs.
set_property PACKAGE_PIN F10 [get_ports {pcie_clk_p}]
set_property PACKAGE_PIN E10 [get_ports {pcie_clk_n}]
create_clock -name pcie_refclk -period 10.0 [get_ports {pcie_clk_p}]

set_property PACKAGE_PIN D9  [get_ports {pcie_rx_p[0]}]
set_property PACKAGE_PIN C9  [get_ports {pcie_rx_n[0]}]
set_property PACKAGE_PIN B10 [get_ports {pcie_rx_p[1]}]
set_property PACKAGE_PIN A10 [get_ports {pcie_rx_n[1]}]
set_property PACKAGE_PIN D7  [get_ports {pcie_tx_p[0]}]
set_property PACKAGE_PIN C7  [get_ports {pcie_tx_n[0]}]
set_property PACKAGE_PIN B6  [get_ports {pcie_tx_p[1]}]
set_property PACKAGE_PIN A6  [get_ports {pcie_tx_n[1]}]

set_property -dict { PACKAGE_PIN F15 IOSTANDARD LVCMOS33 } [get_ports { reset_n }]
set_false_path -from [get_ports {reset_n}]

set_property -dict { PACKAGE_PIN L13 IOSTANDARD LVCMOS33 } [get_ports { led1 }]
set_property -dict { PACKAGE_PIN M13 IOSTANDARD LVCMOS33 } [get_ports { led2 }]
set_property -dict { PACKAGE_PIN K14 IOSTANDARD LVCMOS33 } [get_ports { led3 }]
set_property -dict { PACKAGE_PIN K13 IOSTANDARD LVCMOS33 } [get_ports { led4 }]
set_false_path -to [get_ports {led*}]

set_property CONFIG_VOLTAGE 3.3 [current_design]
set_property CFGBVS VCCO [current_design]
set_property BITSTREAM.CONFIG.SPI_BUSWIDTH 4 [current_design]
