# SPDX-License-Identifier: Apache-2.0
# The board's twelve pins outside the memory and the clock, timed
# (issue 847). Without this, check_timing counted four inputs with no
# input delay and eight outputs with no output delay, and a real
# untimed path could hide among them.

# The asynchronous pins. The reset key, KEY1 and the serial line are
# brought into the design's clock through two flip-flops in the top
# (core_sync and rx_sync); the LEDs and the serial line out are levels
# nothing times against a clock; D02 and D03, the flash's write
# protect and hold, are tied high.
set_false_path -from [get_ports {reset_n key1 uart_rx}]
set_false_path -to [get_ports {led1 led2 led3 led4 uart_tx}]
set_false_path -to [get_ports {flash_d2 flash_d3}]
# The memory's reset, which the memory controller drives as a level
# held for microseconds at power-up and nothing samples against a
# clock; the controller's own constraints leave it untimed. It was the
# eighth output check_timing counted, D02 and D03 being constants it
# does not.
set_false_path -to [get_ports ddr3_reset]

# The configuration flash's SPI (issue 312). FlashPins registers the
# chip's clock, data and select on the design's clock, and the clock
# reaches CCLK through STARTUPE2's USRCCLKO. A master holds each half
# of a bit for at least four cycles (lib/parts/src/cfgflash.rs), so
# the fastest the clock turns is the design's over eight; it is timed
# at that rate.
set flash_reg_c [get_pins -hierarchical \
    -filter {NAME =~ */cfgflash/pins/pclk_reg/C}]
set flash_usrcclko [get_pins -hierarchical \
    -filter {NAME =~ */cfgflash/prim/startupe2/USRCCLKO}]
create_generated_clock -name flash_cclk -source $flash_reg_c \
    -divide_by 8 $flash_usrcclko

# USRCCLKO to the CCLK pin, T_USRCCLKO in the data sheet. DS181's row
# for the Artix-7 -2 was not to hand when this was written; the
# Kintex-7 -2 figure is 0.5 to 6.7 ns, and the maximum here is taken a
# nanosecond wider for that reason.
set t_usr_min 0.5
set t_usr_max 7.5
# The MT25QL128's timing, at its slowest output load. These are the
# bounds this file assumes, set wide of the MT25Q family's published
# figures as the author recalled them, because the data sheet itself
# could not be fetched when this was written: clock low to output
# valid and output hold, data and select setup and hold to the rising
# clock. Confirm them against Micron's MT25QL128 data sheet.
set t_clqv_max 7.0
set t_clqx_min 1.0
set t_su 4.0
set t_h 4.0
# The board's traces, a few centimetres, matched to within this.
set t_trace_max 0.5
set t_trace_min 0.0

# To the chip: the select and the data change on the edge the clock
# falls on and are taken on its rise, four cycles of the design's
# clock later. The chip's clock arrives later than USRCCLKO by
# T_USRCCLKO, which gives setup that much and takes it from hold.
set_output_delay -clock flash_cclk \
    -max [expr {$t_su + $t_trace_max - $t_usr_min}] \
    [get_ports {flash_cs_n flash_d0}]
set_output_delay -clock flash_cclk \
    -min [expr {$t_trace_min - $t_h - $t_usr_max - $t_trace_max}] \
    [get_ports {flash_cs_n flash_d0}]
set_multicycle_path 4 -setup -start \
    -from [get_clocks -of_objects $flash_reg_c] -to flash_cclk
# The hold: the select and the data change no sooner than the clock's
# next fall, four cycles after the rise the chip took them on, so the
# hold is checked against a launch four cycles after that rise. Three
# of the seven bring the hold back to the rise, as the usual pairing
# with a setup of four does; that was all this said at first, which
# told the tools the pins might change on the rise itself and asked
# for 12 ns of route after it, which the router found by a 22 ns
# detour on the select's register and missed timing by (issue 889).
# The one time the select changes near a rise is the guard's cut of
# write enable, which is meant to break the chip's timing.
set_multicycle_path 7 -hold -start \
    -from [get_clocks -of_objects $flash_reg_c] -to flash_cclk

# From the chip: it moves its bit on the falling clock at the pin, and
# the master takes it three cycles of the design's clock after the
# falling edge leaves USRCCLKO.
set_input_delay -clock flash_cclk -clock_fall \
    -max [expr {$t_usr_max + $t_clqv_max + 2 * $t_trace_max}] \
    [get_ports flash_d1]
set_input_delay -clock flash_cclk -clock_fall \
    -min [expr {$t_usr_min + $t_clqx_min + 2 * $t_trace_min}] \
    [get_ports flash_d1]
set_multicycle_path 3 -setup -end \
    -from flash_cclk -to [get_clocks -of_objects $flash_reg_c]
set_multicycle_path 2 -hold -end \
    -from flash_cclk -to [get_clocks -of_objects $flash_reg_c]

# The SD card slot's timing (issues 914 and 929), for the fastest
# divider a program uses, div = 1: 25 MHz. The host (lib/parts/src/sd.rs)
# makes the card's clock itself, from the register `half`, which toggles
# every div + 1 = 2 cycles of the design's clock, so sd_clk is the
# design's clock divided by four, forwarded through its pad.
create_generated_clock -name sd_clk -divide_by 4 \
    -source [get_pins -hierarchical -filter {NAME =~ */sd/half_reg/C}] \
    [get_ports sd_clk]

# The SD Physical Layer Simplified Specification's default-speed
# interface timing: the card takes the host's lines with a setup of
# 5 ns and a hold of 5 ns to the rising clock (tISU, tIH), and drives
# its own within 0 to 14 ns of the falling clock (tODLY). The traces
# from the slot to bank 16 are a few centimetres, matched to within
# half a nanosecond, which each bound is widened by.
set sd_trace 0.5
set_output_delay -clock sd_clk -max [expr {5.0 + $sd_trace}] \
    [get_ports {sd_cmd sd_dat[*]}]
set_output_delay -clock sd_clk -min [expr {-5.0 - $sd_trace}] \
    [get_ports {sd_cmd sd_dat[*]}]
set_input_delay -clock sd_clk -clock_fall -max [expr {14.0 + $sd_trace}] \
    [get_ports {sd_cmd sd_dat[*]}]
set_input_delay -clock sd_clk -clock_fall -min 0.0 \
    [get_ports {sd_cmd sd_dat[*]}]

# The card's lines are taken by input registers in the pads' IOBs, so
# that what lies between a pad and its register is the pad alone.
set_property IOB TRUE [get_cells -hierarchical -filter \
    {NAME =~ */sd/cin_q_reg* || NAME =~ */sd/din_q_reg*}]

# Which edges are real. The host changes its lines a cycle after
# `half` falls, and the card takes them on the next rise, a cycle
# later: the tool's own one-cycle setup, so no multicycle. A value then
# stands until the cycle after the next fall, three cycles after the
# rise that took it, so the hold is three (-start 3), a -30 ns
# requirement; each hold from 0 to 4 was read off the routed design.
# The usual hold, checked on the rise itself, is a change this host
# never makes, and the router answers it with detours on the data
# lines (#889, #914).
set sd_out_regs [get_cells -hierarchical -filter \
    {NAME =~ */sd/cmd_o_reg* || NAME =~ */sd/dat_o_reg* || \
     NAME =~ */sd/cmd_drv_reg* || NAME =~ */sd/dat_drv_reg*}]
set_multicycle_path 3 -hold -start -from $sd_out_regs \
    -to [get_ports {sd_cmd sd_dat[*]}]
# The card drives after a fall, and the input registers take its lines
# three cycles after that fall, a cycle after the next rise: the
# capture is three cycles of the capturing clock late (-end 3). The
# card drives again only after the next fall, a cycle after that
# capture, so the hold is three (-end 3), a -10 ns requirement.
set_multicycle_path 3 -setup -end -from [get_ports {sd_cmd sd_dat[*]}]
set_multicycle_path 3 -hold -end -from [get_ports {sd_cmd sd_dat[*]}]
