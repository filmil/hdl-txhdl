# Run the simulation to its end, then judge it by the testbench's `ok`,
# as //ddr3/sim's script does, for the same reasons.
run -all
set ok [get_value /board_tb/ok]
puts "verdict: $ok"
if {$ok != 1} { exit 1 }
exit 0
