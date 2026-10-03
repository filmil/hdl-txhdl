# SPDX-License-Identifier: Apache-2.0
# The Alinx AX7A200B (xc7a200tfbg484-2), as the a200t examples state it:
# a 200 MHz differential clock, an active-low reset button, four LEDs
# lit when driven low, 3.3 V configuration.
create_clock -add -name sys_clk_p -period 5.0 -waveform {0 2.5} [get_ports {sys_clk_p}]
set_property -dict { PACKAGE_PIN R4 IOSTANDARD DIFF_SSTL15 } [get_ports { sys_clk_p }]
set_property -dict { PACKAGE_PIN T4 IOSTANDARD DIFF_SSTL15 } [get_ports { sys_clk_n }]
set_property -dict { PACKAGE_PIN F15 IOSTANDARD LVCMOS33 } [get_ports { reset_n }]
# KEY1, a user key on the carrier, low while pressed; a second reset.
set_property -dict { PACKAGE_PIN L19 IOSTANDARD LVCMOS33 } [get_ports { key1 }]
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

# The same flash after configuration (issue 312): the select and the
# four data lines are multi-function configuration pins in bank 14,
# which become user pins once the bitstream has loaded. The pins are
# Vivado's own, read off the part with `get_package_pins` by
# function: T19 is IO_L6P_T0_FCS_B_14, P22 IO_L1P_T0_D00_MOSI_14, R22
# IO_L1N_T0_D01_DIN_14, P21 IO_L2P_T0_D02_14, R21 IO_L2N_T0_D03_14.
# The clock is CCLK, L12, which only STARTUPE2 reaches, so it is not
# here.
set_property -dict { PACKAGE_PIN T19 IOSTANDARD LVCMOS33 } [get_ports { flash_cs_n }]
set_property -dict { PACKAGE_PIN P22 IOSTANDARD LVCMOS33 } [get_ports { flash_d0 }]
set_property -dict { PACKAGE_PIN R22 IOSTANDARD LVCMOS33 } [get_ports { flash_d1 }]
set_property -dict { PACKAGE_PIN P21 IOSTANDARD LVCMOS33 } [get_ports { flash_d2 }]
set_property -dict { PACKAGE_PIN R21 IOSTANDARD LVCMOS33 } [get_ports { flash_d3 }]

# The SD card slot (issue 153), J7 on the carrier board, in SD mode.
# The pins are the AX7A200B User Manual REV1.0's, section 3.7: SD_CLK
# E13, SD_CMD E14, SD_DAT0 D15, SD_DAT1 D14, SD_DAT2 F14, SD_DAT3 F13.
# The carrier schematic (AX7A200B Carrier Board Schematic, page 2, the
# connector to the core board, each net's label on its own wire) puts
# them in bank 16, and Vivado's package data for the part agrees,
# read with get_package_pins by function: E13 IO_L4P_T0_16 (SD_CLK on
# B16_L4_P), E14 IO_L4N_T0_16 (SD_CMD, B16_L4_N), D15
# IO_L6N_T0_VREF_16 (SD_DAT0, B16_L6_N), D14 IO_L6P_T0_16 (SD_DAT1,
# B16_L6_P), F14 IO_L1N_T0_16 (SD_DAT2, B16_L1_N), F13 IO_L1P_T0_16
# (SD_DAT3, B16_L1_P).
# Bank 16's VCCO is the core board's VCCIO, which U12, an
# SPX3819M5-3-3, holds at 3.3 V (AC7A200 Core Board Schematic, page 7
# for VCCO_16 on VCCIO, page 10 for U12), so the lines are LVCMOS33,
# as reset_n at F15 in the same bank already is. The slot's 10K
# pull-ups hold a line nobody drives high.
# SD_CD_N, C13 (IO_L8P_T1_16, B16_L8_P), the slot's card detect, is
# not used: the host has no input for it, and a program learns there
# is no card when CMD8 goes unanswered.
set_property -dict { PACKAGE_PIN E13 IOSTANDARD LVCMOS33 } [get_ports { sd_clk }]
set_property -dict { PACKAGE_PIN E14 IOSTANDARD LVCMOS33 } [get_ports { sd_cmd }]
set_property -dict { PACKAGE_PIN D15 IOSTANDARD LVCMOS33 } [get_ports { sd_dat[0] }]
set_property -dict { PACKAGE_PIN D14 IOSTANDARD LVCMOS33 } [get_ports { sd_dat[1] }]
set_property -dict { PACKAGE_PIN F14 IOSTANDARD LVCMOS33 } [get_ports { sd_dat[2] }]
set_property -dict { PACKAGE_PIN F13 IOSTANDARD LVCMOS33 } [get_ports { sd_dat[3] }]

# The data memory lanes stay in block RAM: left to itself the tool
# moves some into distributed RAM to shorten the load-then-jump path.
set_property RAM_STYLE BLOCK [get_cells -hierarchical -regexp {.*lane[0-3]_reg.*}]
