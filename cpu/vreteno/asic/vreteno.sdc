# SPDX-License-Identifier: Apache-2.0
#
# What the core is asked to meet. The clock is 333 MHz, which is three
# times the 100 MHz the Artix-7 board runs it at; a 45 nm standard cell
# library is a faster thing than an FPGA fabric, and the point of
# naming a period at all is that every number after it means something.
#
# The ports are timed as if a chip of the same speed were on the other
# side of each: a fifth of the period to arrive, a fifth to be taken,
# a buffer driving each input, and a load on each output. Ports with no
# timing at all are the usual way a report comes out clean and wrong.

set clk_name core_clock
set clk_port_name clk
set clk_period 3.0
set clk_io_pct 0.2

set clk_port [get_ports $clk_port_name]
create_clock -name $clk_name -period $clk_period $clk_port
set_clock_uncertainty [expr $clk_period * 0.02] [get_clocks $clk_name]

set ins [lsearch -inline -all -not -exact [all_inputs] $clk_port]
set io [expr $clk_period * $clk_io_pct]
set_input_delay $io -clock $clk_name $ins
set_output_delay $io -clock $clk_name [all_outputs]
set_driving_cell -lib_cell BUF_X4 -pin Z $ins
set_load 5.0 [all_outputs]
set_max_fanout 20 [current_design]
