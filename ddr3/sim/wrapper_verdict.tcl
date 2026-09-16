# Run the simulation to its end, then judge it by the testbench's `ok`.
# xsim exits cleanly whatever the design did, and the rule's default
# script stops before it runs, so the verdict has to be read out of the
# design and made the exit status here.
run -all
set ok [get_value /wrapper_tb/ok]
puts "verdict: $ok"
if {$ok != 1} { exit 1 }
exit 0
