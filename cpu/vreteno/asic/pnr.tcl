# SPDX-License-Identifier: Apache-2.0
#
# Vreteno from a netlist of cells to a routed layout, in OpenROAD.
#
# The build names the files in the environment rather than here, so
# that this file is the flow and nothing else. Every step writes what
# it did into the log, and the numbers the document quotes are written
# again, one per line, into the metrics file at the end: a document
# that quotes a number no tool wrote is a document that will be wrong
# one day without anybody noticing.

set platform $::env(TXHDL_PLATFORM)
set top $::env(TXHDL_TOP)

# Pin access, routing and parasitic extraction are parallel, and the
# default is one thread. The build says how many it has.
set_thread_count $::env(TXHDL_THREADS)

proc metric {name value} {
  global metrics
  puts $metrics "$name\t$value"
}

# Where the cells ended up, as two grids over the die: in each square,
# how much of it is filled by cells of any kind, and how much by flip
# flops. The routed design itself is forty megabytes and no document
# can hold it; this is fifty kilobytes and is what a picture of the
# layout is drawn from, by //tools/denmap.
proc write_map {path bins} {
  set block [ord::get_db_block]
  set dbu [$block getDefUnits]
  set die [$block getDieArea]
  set x0 [$die xMin]
  set y0 [$die yMin]
  set w [expr {[$die xMax] - $x0}]
  set h [expr {[$die yMax] - $y0}]
  for {set j 0} {$j < $bins} {incr j} {
    for {set i 0} {$i < $bins} {incr i} {
      set all($i,$j) 0.0
      set ff($i,$j) 0.0
    }
  }
  set n 0
  set nff 0
  set total 0.0
  foreach inst [$block getInsts] {
    set bb [$inst getBBox]
    set cx [expr {([$bb xMin] + [$bb xMax]) / 2.0}]
    set cy [expr {([$bb yMin] + [$bb yMax]) / 2.0}]
    set i [expr {int(($cx - $x0) * $bins / $w)}]
    set j [expr {int(($cy - $y0) * $bins / $h)}]
    if {$i < 0} {set i 0}
    if {$j < 0} {set j 0}
    if {$i >= $bins} {set i [expr {$bins - 1}]}
    if {$j >= $bins} {set j [expr {$bins - 1}]}
    set a [expr {double([$bb getDX]) * double([$bb getDY])}]
    set all($i,$j) [expr {$all($i,$j) + $a}]
    set total [expr {$total + $a}]
    incr n
    if {[regexp {DFF} [[$inst getMaster] getName]]} {
      set ff($i,$j) [expr {$ff($i,$j) + $a}]
      incr nff
    }
  }
  set cell [expr {double($w) * double($h) / ($bins * $bins)}]
  set f [open $path w]
  set wu [format %.2f [expr {double($w) / $dbu}]]
  set hu [format %.2f [expr {double($h) / $dbu}]]
  puts $f "# die\t$wu\t$hu"
  puts $f "# bins\t$bins"
  puts $f "# instances\t$n\t$nff"
  for {set j 0} {$j < $bins} {incr j} {
    set row {}
    set rowff {}
    for {set i 0} {$i < $bins} {incr i} {
      lappend row [format %.3f [expr {$all($i,$j) / $cell}]]
      lappend rowff [format %.3f [expr {$ff($i,$j) / $cell}]]
    }
    puts $f "all\t$j\t[join $row ,]"
    puts $f "ff\t$j\t[join $rowff ,]"
  }
  close $f
  # The cell area, in square microns. It is summed here because it is
  # being summed here anyway, and because `rsz::design_area` answers
  # zero at this point in the flow while `report_design_area` prints
  # the right number.
  metric design_area [format %.1f [expr {$total / ($dbu * $dbu)}]]
}

set metrics [open $::env(TXHDL_METRICS) w]

# ---------------------------------------------------------------- read

# The technology: what the layers are and how wide they may be, then
# the shape and the pins of every cell, then how fast each cell is.
read_lef $platform/lef/NangateOpenCellLibrary.tech.lef
read_lef $platform/lef/NangateOpenCellLibrary.macro.mod.lef
read_liberty $platform/lib/NangateOpenCellLibrary_typical.lib

read_verilog $::env(TXHDL_NETLIST)
link_design $top
read_sdc $::env(TXHDL_SDC)

metric cells [llength [get_cells *]]
metric nets [llength [get_nets *]]

# ----------------------------------------------------------- floorplan

# A square of silicon with room for the cells, the rows they stand in,
# and the tracks they are wired along.
initialize_floorplan -utilization $::env(TXHDL_UTILIZATION) \
  -aspect_ratio 1.0 -core_space 2.0 \
  -site FreePDK45_38x28_10R_NP_162NW_34O
source $platform/make_tracks.tcl

set die [ord::get_die_area]
metric die_width [format %.2f [expr [lindex $die 2] - [lindex $die 0]]]
metric die_height [format %.2f [expr [lindex $die 3] - [lindex $die 1]]]

# The ports become pins on the edge, on the two upper layers that are
# free of the cells below.
place_pins -hor_layers metal5 -ver_layers metal6

# The cells that tie the wells to the supplies, one every 120 microns
# along a row and one at each end of it.
tapcell -distance 120 -tapcell_master TAPCELL_X1 -endcap_master TAPCELL_X1

# The power grid: the rails the rows sit on, and the straps above them.
source $platform/grid_strategy-M1-M4-M7.tcl
pdngen

# What a wire costs per micron, so that a delay is a delay and not a
# guess about a wire that has not been drawn yet.
source $platform/setRC.tcl

# ---------------------------------------------------------------- place

global_placement -density $::env(TXHDL_DENSITY) -pad_left 2 -pad_right 2
estimate_parasitics -placement
repair_design
detailed_placement
check_placement -verbose

# ------------------------------------------------------------------ cts

# The clock reaches every register through a tree of buffers, and the
# registers move a little to make that tree shorter.
clock_tree_synthesis -buf_list BUF_X4 -root_buf BUF_X4 -sink_clustering_enable
set_propagated_clock [all_clocks]
estimate_parasitics -placement
repair_clock_nets
detailed_placement

# ---------------------------------------------------------------- route

global_route -congestion_iterations 30
estimate_parasitics -global_routing
repair_timing -setup
detailed_placement

# No iteration limit. Five passes left fifteen shorts in one congested
# corner, which is fifteen too many: a routed design with violations
# is a design that has not been routed.
detailed_route -output_drc $::env(TXHDL_DRC) -verbose 0
estimate_parasitics -global_routing

# How much of the core the design takes, and where its cells are.
# Both are read here, before the fill, because a filled core is a full
# one by construction and neither number would say anything after it.
report_design_area
metric utilization [format %.1f [expr [rsz::utilization] * 100]]
write_map $::env(TXHDL_MAP) 64

# ----------------------------------------------------------------- fill

# What is left of a row after the cells that do something: filler
# cells, which carry the wells and the supply rails across the gaps.
# They come last because they have no signal pins and therefore
# nothing to route.
filler_placement {FILLCELL_X1 FILLCELL_X2 FILLCELL_X4 FILLCELL_X8 \
                  FILLCELL_X16 FILLCELL_X32}
check_placement

# --------------------------------------------------------------- report

report_worst_slack -max
report_worst_slack -min
report_tns
report_checks -path_delay max -fields {slew cap input nets fanout} -digits 3
report_power
report_check_types -max_slew -max_capacitance -max_fanout -violators
report_clock_skew

metric worst_slack_max [format %.4f [sta::worst_slack -max]]
metric worst_slack_min [format %.4f [sta::worst_slack -min]]
metric tns [format %.4f [sta::total_negative_slack -max]]
set period [get_property [lindex [get_clocks *] 0] period]
metric clock_period [format %.3f $period]

# Every violation the routed design still has. A count of zero is the
# only interesting value, and it is the one this flow is checked on.
set drc_count 0
if {[file exists $::env(TXHDL_DRC)]} {
  set f [open $::env(TXHDL_DRC) r]
  set drc_count [regexp -all {violation type} [read $f]]
  close $f
}
metric drc_violations $drc_count

write_def $::env(TXHDL_DEF)
write_verilog $::env(TXHDL_ROUTED)

close $metrics
exit 0
