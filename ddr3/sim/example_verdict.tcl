# Run AMD's example testbench to its end, then judge it as it judges
# itself: the memory calibrated, and the traffic generator's compare
# never failed. xsim exits cleanly whatever the design did, and the
# rule's default script stops before it runs, so the verdict has to be
# read out of the design and made the exit status here.
run -all
set calib [get_value /sim_tb_top/init_calib_complete]
set error [get_value /sim_tb_top/tg_compare_error]
puts "verdict: calibrated $calib, compare error $error"
if {$calib != 1 || $error != 0} { exit 1 }
exit 0
