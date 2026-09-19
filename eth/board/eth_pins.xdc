# SPDX-License-Identifier: Apache-2.0
# The JL2121-N040I PHY's pins on the Alinx AX7A200B, from the table of
# the manual's section on the gigabit Ethernet interface, and the clock
# the PHY recovers from the line. Every design that drives this port
# reads this file: //eth's echo, and //flagship.
#
# The I/O standard is LVCMOS33 as for the board's other user pins; the
# manual does not state it, and the port works with it.

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
