# SPDX-License-Identifier: Apache-2.0
# The Ethernet echo on the Alinx AX7A200 (xc7a200tfbg484-2).
#
# The clock and the LEDs are the ones cpu/vreteno/board/ax7a200.xdc
# names. The Ethernet pins are the JL2121-N040I PHY's, from the table
# of the AX7A200B manual's section on the gigabit Ethernet interface.
# The I/O standard of the Ethernet pins is LVCMOS33 as for the board's
# other user pins; the manual does not state it, and it is to be
# confirmed on the board.

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

# The PHY's receive clock, 125 MHz at gigabit.
create_clock -add -name eth_rxck -period 8.0 [get_ports {eth_rxck}]
set_clock_groups -asynchronous \
  -group [get_clocks -include_generated_clocks sys_clk_p] \
  -group [get_clocks eth_rxck]

# Transmit.
set_property PACKAGE_PIN P15 [get_ports {eth_txck}]
set_property PACKAGE_PIN N14 [get_ports {eth_txd[0]}]
set_property PACKAGE_PIN P16 [get_ports {eth_txd[1]}]
set_property PACKAGE_PIN R17 [get_ports {eth_txd[2]}]
set_property PACKAGE_PIN R16 [get_ports {eth_txd[3]}]
set_property PACKAGE_PIN N17 [get_ports {eth_txctl}]

# Receive.
set_property PACKAGE_PIN V18 [get_ports {eth_rxck}]
set_property PACKAGE_PIN P19 [get_ports {eth_rxd[0]}]
set_property PACKAGE_PIN U18 [get_ports {eth_rxd[1]}]
set_property PACKAGE_PIN U17 [get_ports {eth_rxd[2]}]
set_property PACKAGE_PIN P17 [get_ports {eth_rxd[3]}]

# The PHY's other pins: the receive control line, the management
# interface, and the reset.
set_property PACKAGE_PIN R19 [get_ports {eth_rxctl}]
set_property PACKAGE_PIN N13 [get_ports {eth_mdc}]
set_property PACKAGE_PIN P14 [get_ports {eth_mdio}]
set_property PACKAGE_PIN R14 [get_ports {eth_reset_n}]

# Every Ethernet pin, to be confirmed on the board.
set_property IOSTANDARD LVCMOS33 [get_ports {eth_*}]

set_property CONFIG_VOLTAGE 3.3 [current_design]
set_property CFGBVS VCCO [current_design]
