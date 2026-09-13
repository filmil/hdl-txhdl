# SPDX-License-Identifier: Apache-2.0
# For synthesis of the Vreteno core: the clock, and nothing else, since
# the question is whether the netlist synthesizes and to what, not
# where its pins go on a board.
create_clock -period 10 -name clk [get_ports clk]
# The data memory's four lanes are block RAM: left to itself the tool
# moves three of them into distributed RAM to shorten the load-then-jump
# path, and the point of the third stage was that they need not be.
set_property RAM_STYLE BLOCK [get_cells -hierarchical -regexp {.*lane[0-3]_reg.*}]
