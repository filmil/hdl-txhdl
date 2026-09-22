# SPDX-License-Identifier: Apache-2.0
# The board's bus from the JTAG cable (issue 241): through the master
# in `vreteno_board_jtag.v`, words are written into the data memory and
# the DDR3 and read back, and the timer is read twice to show a
# peripheral answering while the core runs. Run from Vivado's hardware
# manager against the board, with the hardware server's address as
# `HOSTPORT` in the environment, or localhost:3122 by default:
#
#   bazel run //cpu/vreteno:vreteno_board_jtag_probe
#
# Every line that matters begins with `jtag:` so a reader can grep
# for it; the script exits 1 on the first word that comes back wrong.
set hostport [expr {[info exists ::env(HOSTPORT)] ? $::env(HOSTPORT) : "localhost:3122"}]
open_hw_manager
connect_hw_server -url $hostport
current_hw_target [get_hw_targets */xilinx_tcf/Digilent/*]
open_hw_target
set dev [lindex [get_hw_devices] 0]
current_hw_device $dev
refresh_hw_device $dev
set axi [lindex [get_hw_axis -of_objects $dev] 0]
if { $axi == "" } {
    puts "jtag: no AXI master on the part; is the JTAG bitstream loaded?"
    exit 1
}
puts "jtag: master $axi"

# One word written and read back, at an address; the address and the
# data are hexadecimal without a prefix, as the hardware manager takes
# them.
proc write_word {axi addr data} {
    delete_hw_axi_txn -quiet [get_hw_axi_txns -quiet wr]
    create_hw_axi_txn wr $axi -type write -address $addr -data $data -len 1
    run_hw_axi wr
}
proc read_word {axi addr} {
    delete_hw_axi_txn -quiet [get_hw_axi_txns -quiet rd]
    create_hw_axi_txn rd $axi -type read -address $addr -len 1
    run_hw_axi rd
    return [string tolower [lindex [get_property DATA [get_hw_axi_txns rd]] 0]]
}
proc check {axi name addr data} {
    write_word $axi $addr $data
    set back [read_word $axi $addr]
    if { $back != $data } {
        puts "jtag: $name at $addr: wrote $data, read $back: BAD"
        exit 1
    }
    puts "jtag: $name at $addr: wrote $data, read $back: ok"
}

# The data memory, the DDR3, and a second word of each, so that an
# answer is not one lucky value.
check $axi "data memory" 00001010 c0ffee11
check $axi "data memory" 00001014 5eed0241
check $axi "DDR3" 40000100 deadbeef
check $axi "DDR3" 40000104 0badf00d

# The timer's low word, `mtime` in the CLINT, twice: two different numbers, which is a
# peripheral answering a read while the core does whatever it does.
set t0 [read_word $axi 0200bff8]
set t1 [read_word $axi 0200bff8]
if { $t0 == $t1 } {
    puts "jtag: timer at 0200bff8 read $t0 twice: not counting: BAD"
    exit 1
}
puts "jtag: timer at 0200bff8 read $t0 then $t1: counting: ok"
puts "jtag: every word came back: ok"
close_hw_target
