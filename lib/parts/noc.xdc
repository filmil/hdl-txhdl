# SPDX-License-Identifier: Apache-2.0
# For synthesis of one node of the network, and of one switch inside
# it. A node holds no register: the flip-flops that bound its path are
# the channels' on either side of it, and they belong to whatever
# instantiates the node. So the question is the node's own delay from
# an input to an output, which `set_max_delay` asks for, against a
# ten-nanosecond budget (issue 99).
create_clock -period 10 -name clk [get_ports clk]
set_max_delay -from [all_inputs] -to [all_outputs] 10
