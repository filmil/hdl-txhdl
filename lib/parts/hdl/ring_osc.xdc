# SPDX-License-Identifier: Apache-2.0
#
# The ring oscillators of `ring_osc.v` are loops of gates, which the
# timer cannot close and the design rule check refuses. Both are told
# so: the loop check becomes a warning, and no path through a ring's
# chain is timed, since its period is the ring's own and not the
# clock's. The samplers' flops stay timed to the clock as any flop is.
#
# The rings are found by their cells, whose names `ring_osc.v` gives
# them, and not by the name of the instance a design gives the module:
# each ring is a LUT2 `gate` and LUT1 inverters `not_j`, under generate
# blocks called `ring`, so a board that names the instance `ring`, as
# every board here does, is matched as one that names it `ring_osc`
# would be (issue 751). The pattern has no brackets, since `=~` reads
# `[..]` as a class of characters. A path through any of their outputs
# is a path through a chain.
set_property SEVERITY {Warning} [get_drc_checks LUTLP-1]
set_false_path -through [get_pins -of_objects [get_cells -hierarchical -filter {(REF_NAME == LUT2 && NAME =~ *ring*.gate) || (REF_NAME == LUT1 && NAME =~ *ring*.inv*.not_j)}] -filter {DIRECTION == OUT}]
