# SPDX-License-Identifier: Apache-2.0
# For the Sv32 unit out of context (issue 1014): the core's clock, and
# the request as the core gives it, from a register of its own. One
# nanosecond stands for that register's clock to output and the wire to
# the unit, so a path from a request port into the answer's registers
# is timed against the nine that are left. The answers leave from
# registers, so what follows them is the core's to time.
create_clock -period 10 -name clk [get_ports clk]
set_input_delay -clock clk 1.0 [get_ports -filter {NAME =~ "ireq*" || NAME =~ "dreq*" || NAME =~ "satp*" || NAME =~ "prv*" || NAME =~ "sum" || NAME =~ "mxr" || NAME =~ "flush"}]
