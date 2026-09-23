# The run, and the verdict read from the board's `ok`, which the test
# sets when the link came up and every word read back is the one the
# design keeps.
run -all
set ok [get_value /board/ok]
puts "verdict: $ok"
if {$ok != 1} { exit 1 }
exit 0
