# SPDX-License-Identifier: Apache-2.0
# The SiI9134 encoder's pins on the Alinx AX7A200B, from the table of
# the manual's section on the HDMI output. Every design that drives the
# encoder reads this file: //hdmi's demonstration, and //flagship.
#
# The I/O standard is LVCMOS33 as for the board's other user pins, and
# red is on the top eight bits; both were confirmed on the board by the
# test picture, which came up with its bars in the colours the netlist
# names.

# The encoder's reset, clock, syncs and enable.
set_property PACKAGE_PIN Y17 [get_ports {hdmi_nreset}]
# The chip's reset again, on the ball an earlier revision of this board
# wires it to. The vendor's demonstration drives both, and this design
# follows it rather than guessing which revision is on the desk.
set_property PACKAGE_PIN L18 [get_ports {hdmi_nreset_alt}]
set_property PACKAGE_PIN Y22 [get_ports {hdmi_clk}]
set_property PACKAGE_PIN T18 [get_ports {hdmi_hs}]
set_property PACKAGE_PIN R18 [get_ports {hdmi_vs}]
set_property PACKAGE_PIN U22 [get_ports {hdmi_de}]

# The encoder's 24 data inputs, D[0] to D[23].
set_property PACKAGE_PIN V22 [get_ports {hdmi_d[0]}]
set_property PACKAGE_PIN Y18 [get_ports {hdmi_d[1]}]
set_property PACKAGE_PIN Y19 [get_ports {hdmi_d[2]}]
set_property PACKAGE_PIN W19 [get_ports {hdmi_d[3]}]
set_property PACKAGE_PIN W20 [get_ports {hdmi_d[4]}]
set_property PACKAGE_PIN Y21 [get_ports {hdmi_d[5]}]
set_property PACKAGE_PIN U21 [get_ports {hdmi_d[6]}]
set_property PACKAGE_PIN T21 [get_ports {hdmi_d[7]}]
set_property PACKAGE_PIN W21 [get_ports {hdmi_d[8]}]
set_property PACKAGE_PIN W22 [get_ports {hdmi_d[9]}]
set_property PACKAGE_PIN T20 [get_ports {hdmi_d[10]}]
set_property PACKAGE_PIN AB18 [get_ports {hdmi_d[11]}]
set_property PACKAGE_PIN AA18 [get_ports {hdmi_d[12]}]
set_property PACKAGE_PIN AA19 [get_ports {hdmi_d[13]}]
set_property PACKAGE_PIN AB20 [get_ports {hdmi_d[14]}]
set_property PACKAGE_PIN AA20 [get_ports {hdmi_d[15]}]
set_property PACKAGE_PIN AA21 [get_ports {hdmi_d[16]}]
set_property PACKAGE_PIN AB22 [get_ports {hdmi_d[17]}]
set_property PACKAGE_PIN AB21 [get_ports {hdmi_d[18]}]
set_property PACKAGE_PIN W17 [get_ports {hdmi_d[19]}]
set_property PACKAGE_PIN V17 [get_ports {hdmi_d[20]}]
set_property PACKAGE_PIN V20 [get_ports {hdmi_d[21]}]
set_property PACKAGE_PIN U20 [get_ports {hdmi_d[22]}]
set_property PACKAGE_PIN V19 [get_ports {hdmi_d[23]}]

# The configuration bus.
set_property PACKAGE_PIN H13 [get_ports {hdmi_scl}]
set_property PACKAGE_PIN G13 [get_ports {hdmi_sda}]

# Every encoder pin, to be confirmed on the board.
set_property IOSTANDARD LVCMOS33 [get_ports {hdmi_*}]
