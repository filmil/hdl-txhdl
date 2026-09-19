# SPDX-License-Identifier: Apache-2.0
#
# Where the flagship's subsystems landed on the die, read off the routed
# checkpoint and written as a TSV on standard output.
#
# The design is four subsystems that share a part and almost nothing
# else, so the question the map answers is whether the placer kept them
# apart, and where the memory controller's pins pulled the core. None of
# that is visible in a utilisation report, which counts but does not
# say where.
#
# A cell is attributed to a subsystem by the top-level instance it is
# under, which is exactly how flagship.v is written: one instance per
# subsystem, named in the top. The memory controller is split out from
# the rest of the core's netlist, because it is the largest single
# thing in the design and it is pinned to the memory's I/O bank.
#
# Only cells in SLICE sites go into the map. They are the logic, and
# their coordinates are one grid: a slice holds several cells, so these
# are counts of cells rather than of slices, and they are larger than a
# utilisation report's. The block RAMs and the DSPs are counted in the
# totals instead, where their own coordinate systems do not have to be
# reconciled with anything.
#
#   bazel run //flagship:place_report > docs/flagship_place.tsv

# Which subsystem a cell belongs to, by its hierarchical name. First
# match wins, so the memory controller is tested before the core it
# sits inside.
proc subsystem {name} {
  if {[string match "lowered/ddr3*" $name]} { return "memory" }
  if {[string match "lowered/*" $name]}     { return "core" }
  if {[string match "rgmii/*" $name]}       { return "ethernet" }
  if {[string match "mac_rx/*" $name]}      { return "ethernet" }
  if {[string match "mac_tx/*" $name]}      { return "ethernet" }
  if {[string match "*_crossing/*" $name]}  { return "ethernet" }
  if {[string match "video/*" $name]}       { return "video" }
  if {[string match "master/*" $name]}      { return "video" }
  if {[string match "v*_cdc/*" $name]}      { return "crossing" }
  return "top"
}

# The checkpoint, wherever the runfiles put it.
set dcp "flagship/flagship_pnr.pnr.dcp"
if {![file exists $dcp]} { set dcp "flagship_pnr.pnr.dcp" }
open_checkpoint $dcp

# Every placed primitive, its name and its site, as two lists: one
# call each rather than two per cell, which is what makes this seconds
# rather than minutes.
set placed [get_cells -hier -filter {IS_PRIMITIVE == 1 && LOC != ""}]
set names [get_property NAME $placed]
set locs [get_property LOC $placed]

# The bins of the map, and the totals per subsystem.
array set bin {}
array set cells {}
array set brams {}
array set dsps {}
array set minx {}
array set maxx {}
array set miny {}
array set maxy {}
set gx 0
set gy 0

foreach name $names loc $locs {
  set who [subsystem $name]
  if {[regexp {^SLICE_X(\d+)Y(\d+)$} $loc -> x y]} {
    incr cells($who)
    incr bin($who,$x,$y)
    if {![info exists minx($who)] || $x < $minx($who)} { set minx($who) $x }
    if {![info exists maxx($who)] || $x > $maxx($who)} { set maxx($who) $x }
    if {![info exists miny($who)] || $y < $miny($who)} { set miny($who) $y }
    if {![info exists maxy($who)] || $y > $maxy($who)} { set maxy($who) $y }
    if {$x > $gx} { set gx $x }
    if {$y > $gy} { set gy $y }
  } elseif {[string match "RAMB*" $loc]} {
    incr brams($who)
  } elseif {[string match "DSP*" $loc]} {
    incr dsps($who)
  }
}

puts "# part\t[get_property PART [current_design]]"
puts "# grid\t[expr {$gx + 1}]\t[expr {$gy + 1}]"
puts "#"
puts "# who\tcells\tbrams\tdsps\tx0\tx1\ty0\ty1"
foreach who [lsort [array names cells]] {
  set b 0
  set d 0
  if {[info exists brams($who)]} { set b $brams($who) }
  if {[info exists dsps($who)]} { set d $dsps($who) }
  puts "total\t$who\t$cells($who)\t$b\t$d\t$minx($who)\t$maxx($who)\t$miny($who)\t$maxy($who)"
}

puts "#"
puts "# who\tslice x\tslice y\tcells"
foreach key [lsort [array names bin]] {
  lassign [split $key ,] who x y
  puts "cell\t$who\t$x\t$y\t$bin($key)"
}

# Vivado stays at its prompt when a script ends, and this one is run
# without a terminal, so it says when it is done.
exit
