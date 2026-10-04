# SPDX-License-Identifier: Apache-2.0
# For synthesis of Razboj's rasteriser out of context: the clock, at the
# 100 MHz the flagship's bus runs at, and nothing else, since the
# question is what the netlist synthesises to and whether it meets the
# clock, not where its pins go on a board.
create_clock -period 10 -name clk [get_ports clk]
