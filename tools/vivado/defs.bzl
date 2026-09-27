# SPDX-License-Identifier: Apache-2.0
"""Synthesis that fails on the warnings which produce wrong hardware.

Vivado answers some mistakes with a warning and then synthesises
something that is not what was written. A green build, met timing and a
bitstream that cannot work: the only trace is one line in a log nobody
reads, and the cost is a place and route and a run on the board.

`vivado_synthesis2` here is `rules_vivado`'s rule with a check added
after `synth_design`, asking Vivado how many times it issued each of
those messages. Any of them, and synthesis fails before the netlist is
written.

Load this in place of `@rules_vivado//build/vivado:rules.bzl` for
synthesis. The other Vivado rules are unaffected and still come from
there; only the rule that elaborates RTL needs this.

`vivado_place_and_route2` here is the same idea after `route_design`:
a routed design whose worst setup or hold slack is negative fails the
build (issue 759). A bitstream that misses timing is a bitstream that
works on some boards and some days; the flagship missed by 1.9 ns
after #501 and nothing said so, since every place and route target
writes its timing summary and its bitstream whatever the slack is.
"""

load(
    "@rules_vivado//build/vivado:rules.bzl",
    _vivado_place_and_route2 = "vivado_place_and_route2",
    _vivado_synthesis2 = "vivado_synthesis2",
)

# Message IDs that mean the hardware is not what the source says.
#
# Not every Vivado warning belongs here. One earns its place by being
# silent at every later step: synthesis keeps going, timing is met,
# the bitstream builds, and the design is still wrong. A warning that
# fails, or that shows up as a timing violation, is already visible.
FATAL_SYNTH_MESSAGES = {
    # A sized decimal literal too large for its width is cut to fit,
    # and a comparison against it can become one that is never true.
    # Found on the flagship top, where a reset detector never fired:
    # `21'd2_100_000` became 2_848. Issue #353.
    "Synth 8-10929": "a sized literal was truncated to fit its width",
}

def _fatal_message_checks():
    """Tcl that fails the run if any fatal message was issued.

    `get_msg_config -count` is asked after `synth_design`, rather than
    the severity being raised before it, because the rule offers a hook
    after synthesis and none before. The difference is when it stops,
    not whether: nothing downstream runs either way, because the error
    comes before `write_checkpoint`.

    The count is read into a variable on a line of its own instead of
    inside the `if`. If the query is ever wrong -- a renamed option, a
    Vivado that answers differently -- that line fails with Vivado's
    own complaint about it, which says so. Folded into the condition it
    would read as a design that passed the check.
    """
    lines = []
    for i, (id, what) in enumerate(FATAL_SYNTH_MESSAGES.items()):
        var = "txhdl_fatal_%d" % i
        lines.append("set %s [get_msg_config -id {%s} -count]" % (var, id))
        lines.append(
            'if {$%s > 0} { error "%s: %s. Vivado carried on and ' % (var, id, what) +
            "synthesised something else, so the design is not what the " +
            'source says. Search the synthesis log for %s." }' % id,
        )
    return lines

def vivado_synthesis2(name, post_synth_design = None, **kwargs):
    """`rules_vivado`'s synthesis, failing on the warnings in FATAL_SYNTH_MESSAGES.

    Args:
      name: the target name.
      post_synth_design: Tcl to run after `synth_design`, as upstream.
        The checks are appended after it, so a target's own Tcl still
        sees the design as synthesis left it.
      **kwargs: passed to `rules_vivado`'s rule unchanged.
    """
    _vivado_synthesis2(
        name = name,
        post_synth_design = (post_synth_design or []) + _fatal_message_checks(),
        **kwargs
    )

def _timing_checks():
    """Tcl that fails the run if the routed design misses timing.

    It runs after `route_design` and before the rule writes its
    reports, since that is where the rule's hook is, so on a failure it
    writes the timing summary and the worst paths into the log itself:
    the report file a passing run leaves is not written by a failing
    one, and the log is what a failed action prints.

    Each slack is read into a variable on a line of its own, for the
    reason `_fatal_message_checks` gives. A design with no constrained
    path of a kind answers with nothing, which is not a failure.
    """
    return [
        "set txhdl_wns [get_property SLACK [get_timing_paths -delay_type max]]",
        "set txhdl_whs [get_property SLACK [get_timing_paths -delay_type min]]",
        'set txhdl_miss [expr {($txhdl_wns ne "" && $txhdl_wns < 0) || ' +
        '($txhdl_whs ne "" && $txhdl_whs < 0)}]',
        "if {$txhdl_miss} { report_timing_summary -max_paths 5 }",
        'if {$txhdl_miss} { error "timing is not met: the worst setup ' +
        "slack is $txhdl_wns ns and the worst hold slack $txhdl_whs ns, " +
        "and a bitstream that misses timing is not one to program. The " +
        'timing summary and the worst paths are in the log above." }',
    ]

def vivado_place_and_route2(name, post_route_design = None, **kwargs):
    """`rules_vivado`'s place and route, failing when timing is not met.

    Args:
      name: the target name.
      post_route_design: Tcl to run after `route_design`, as upstream.
        The check is appended after it, so a target's own Tcl runs on
        the routed design first.
      **kwargs: passed to `rules_vivado`'s rule unchanged.
    """
    _vivado_place_and_route2(
        name = name,
        post_route_design = (post_route_design or []) + _timing_checks(),
        **kwargs
    )
