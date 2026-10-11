# SPDX-License-Identifier: Apache-2.0
# ex_smul's products out of context (issue 1649): the core's ten
# nanosecond clock, and the operands as a register elsewhere gives them,
# one nanosecond standing for its clock to output and the wire, so a
# path from an operand into a product's register is timed against the
# nine that are left.
create_clock -period 10 -name clk [get_ports clk]
set_input_delay -clock clk 1.0 [get_ports -filter {NAME =~ "a*" || NAME =~ "b*" || NAME =~ "h*" || NAME =~ "c*" || NAME =~ "d*"}]
