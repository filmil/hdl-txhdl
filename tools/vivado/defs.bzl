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
"""

load(
    "@rules_vivado//build/vivado:rules.bzl",
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
