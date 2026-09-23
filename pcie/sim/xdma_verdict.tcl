# The run, and the verdict read from the testbench's `ok`, as the
# board's simulation does: the run is what the testbench holds, and
# the value is what says it went.
run -all
set ok [get_value /xdma_tb/ok]
puts "verdict: $ok"
if {$ok != 1} { exit 1 }
exit 0
