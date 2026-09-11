# For synthesis of the Vreteno core: the clock, and nothing else, since
# the question is whether the netlist synthesizes and to what, not
# where its pins go on a board.
create_clock -period 10 -name clk [get_ports clk]
