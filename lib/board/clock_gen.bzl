# SPDX-License-Identifier: Apache-2.0
"""A clock manager for a board top, planned at build time (issue 304).

    load("//lib/board:clock_gen.bzl", "clock_gen")

    clock_gen(
        name = "clk_pix",
        primitive = "mmcm",
        input_hz = 200000000,
        # Each output a frequency in hertz and its tolerance in parts per
        # million, nought for exactly.
        outputs = ["25175000:1000"],
    )

writes `clk_pix.v`, the module `clk_pix(clk_in, rst, clk_0.., locked)`
that //tools/clockgen planned, and `clk_pix.txt`, the plan: the settings
and what each output gets. The build fails, naming the outputs, when no
setting meets them all. Output `k` is the primitive's `CLKOUT<k>`, so of an
MMCM's only the first may divide in eighths: list first the output that
needs them. With `testbench = True` it writes
`clk_pix_tb.v` as well, which measures each output in simulation.
"""

def clock_gen(name, primitive, input_hz, outputs, testbench = False, whole = False, **kwargs):
    """Plans the clock manager `name` and writes its module.

    Args:
      name: the module's name and the rule's.
      primitive: "mmcm" or "pll".
      input_hz: the input clock's frequency in hertz.
      outputs: each "hz:ppm".
      testbench: whether to write `<name>_tb.v` as well.
      whole: whether an MMCM keeps to whole multipliers and dividers, as a
        PLL does, so no fractional divider's jitter, whatever oscillator
        that leaves (#1830).
      **kwargs: the genrule's other attributes, such as `tags`.
    """
    outs = [name + ".v", name + ".txt"]
    tb = ""
    if testbench:
        outs.append(name + "_tb.v")
        tb = " --testbench $(location %s_tb.v)" % name
    if whole:
        tb += " --whole"
    native.genrule(
        name = name,
        outs = outs,
        cmd = ("$(location //tools/clockgen) --name %s --primitive %s " +
               "--input-hz %s %s --verilog $(location %s.v) " +
               "--report $(location %s.txt)%s > /dev/null") % (
            name,
            primitive,
            input_hz,
            " ".join(["--out " + o for o in outputs]),
            name,
            name,
            tb,
        ),
        tools = ["//tools/clockgen"],
        **kwargs
    )
