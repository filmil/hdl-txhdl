# SPDX-License-Identifier: Apache-2.0
# The debug module from the JTAG cable (issue 154): through the master
# in `vreteno_board_jtag.v`, the module at 0x1000_0000 is told to halt
# the core, its status is read, the core's stack pointer and `dpc` are
# read by the abstract command, and the core is resumed. Run from
# Vivado's hardware manager against the board, with the hardware
# server's address as `HOSTPORT` in the environment, or localhost:3122
# by default, with any program running on the core:
#
#   bazel run //cpu/vreteno:vreteno_board_dm_probe
#
# Every line that matters begins with `dm:` so a reader can grep for
# it; the script exits 1 on the first thing that comes back wrong.
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
    puts "dm: no AXI master on the part; is the JTAG bitstream loaded?"
    exit 1
}
puts "dm: master $axi"

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
# The module's registers: four bytes a number, from 0x1000_0000.
proc dm_addr {number} { return [format %08x [expr {0x10000000 + 4 * $number}]] }
set data0 [dm_addr 0x04]
set dmcontrol [dm_addr 0x10]
set dmstatus [dm_addr 0x11]
set abstractcs [dm_addr 0x16]
set command [dm_addr 0x17]
# The access-register command for one 32-bit register.
proc access {regno write} { return [format %08x [expr {(2 << 20) | (1 << 17) | ($write << 16) | $regno}]] }
proc expect_bits {axi name addr mask want} {
    set got [read_word $axi $addr]
    if { ([expr 0x$got] & $mask) != $want } {
        puts "dm: $name read $got, wanted [format %08x $want] under [format %08x $mask]: BAD"
        exit 1
    }
    puts "dm: $name read $got: ok"
}
proc regread {axi name regno} {
    global command data0 abstractcs
    write_word $axi $command [access $regno 0]
    set cs [read_word $axi $abstractcs]
    if { ([expr 0x$cs] & 0x700) != 0 } {
        puts "dm: $name: command refused, abstractcs $cs: BAD"
        exit 1
    }
    set v [read_word $axi $data0]
    puts "dm: $name (regno [format %04x $regno]) = $v"
    return $v
}

# Active, running.
write_word $axi $dmcontrol 00000001
expect_bits $axi "dmstatus, running" $dmstatus 0x00000cff 0x00000c82
# Halt, and see it halted.
write_word $axi $dmcontrol 80000001
expect_bits $axi "dmstatus, halted" $dmstatus 0x00000fff 0x00000382
# The stack pointer, the return address and dpc, by the abstract command.
regread $axi "x2, sp" 0x1002
regread $axi "x1, ra" 0x1001
set dpc [regread $axi "dpc" 0x7b1]
set dcsr [regread $axi "dcsr" 0x7b0]
if { ([expr 0x$dcsr] & 0x1c0) != 0x0c0 } {
    puts "dm: dcsr cause is not the halt request: BAD"
    exit 1
}
puts "dm: dcsr cause 3, the halt request: ok"
# Write dpc back to itself, so the resume goes where the core was.
write_word $axi $data0 $dpc
write_word $axi $command [access 0x7b1 1]
expect_bits $axi "abstractcs after the write" $abstractcs 0x00000700 0
# Withdraw the halt request, resume, and see it running and acknowledged.
write_word $axi $dmcontrol 00000001
write_word $axi $dmcontrol 40000001
expect_bits $axi "dmstatus, resumed" $dmstatus 0x00030fff 0x00030c82
puts "dm: halt, registers, resume: every step ok"
close_hw_target
