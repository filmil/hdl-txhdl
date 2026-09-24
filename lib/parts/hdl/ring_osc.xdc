# SPDX-License-Identifier: Apache-2.0
#
# The ring oscillators of `ring_osc.v` are loops of gates, which the
# timer cannot close and the design rule check refuses. Both are told
# so: the loop check becomes a warning, and no path through a ring's
# chain is timed, since its period is the ring's own and not the
# clock's. The samplers' flops stay timed to the clock as any flop is.
set_property SEVERITY {Warning} [get_drc_checks LUTLP-1]
set_false_path -through [get_nets -hierarchical -filter {NAME =~ *ring_osc*chain*}]
