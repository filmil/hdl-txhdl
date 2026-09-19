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

set_property CONFIG_VOLTAGE 3.3 [current_design]
set_property CFGBVS VCCO [current_design]

# The board has four LEDs and this design names three, so K13 is an
# unused pin, and a 7 series bitstream pulls unused pins down by
# default, which leaves that LED glowing faintly. See issue #232.
set_property BITSTREAM.CONFIG.UNUSEDPIN PULLNONE [current_design]
