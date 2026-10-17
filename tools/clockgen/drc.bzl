# SPDX-License-Identifier: Apache-2.0
"""A test of one edge of the clock managers' limits (issue 304).

A case writes a module with the settings given, nothing checked, puts
it through Vivado to a bitstream as a top of its own, and expects the
bitstream to be written or refused. Vivado checks a clock manager's
ranges only among a bitstream's checks (PDRC-34 and its like, on the
oscillator, the phase detector and the outputs), not after synthesis,
placement or routing, so the case goes all the way. Building the target
is the test: its Tcl fails when the outcome is not the one expected.
Manual, as every Vivado target is.
"""

load("@rules_vivado//build/vivado:rules.bzl", "vivado_synthesis2")

def clock_drc_case(name, primitive, input_hz, settings, accepted):
    """One limit's edge, just inside it (`accepted`) or just outside.

    Args:
      name: the case's name, which is also the module's.
      primitive: "mmcm" or "pll".
      input_hz: the input clock's frequency, as CLKIN1_PERIOD says it.
      settings: "D,M8,O8", the dividers in eighths.
      accepted: whether Vivado must write the bitstream.
    """
    native.genrule(
        name = name + "_v",
        outs = [name + ".v"],
        cmd = ("$(location //tools/clockgen) --name %s --primitive %s " +
               "--input-hz %s --settings %s --verilog $@") % (
            name,
            primitive,
            input_hz,
            settings,
        ),
        tags = ["manual"],
        tools = ["//tools/clockgen"],
    )
    expect = (
        "if {$refused} {error \"clockgen: %s is inside the limits and was refused: $err\"}" % name
    ) if accepted else (
        "if {!$refused} {error \"clockgen: %s is outside the limits and was accepted\"}" % name
    )
    vivado_synthesis2(
        name = name,
        srcs = [":" + name + "_v"],
        part = "xc7a200tfbg484-2",
        top = name,
        tags = ["manual"],
        post_synth_design = [
            # The module's ports have no pins; that is not the question.
            "set_property SEVERITY {Warning} [get_drc_checks {NSTD-1 UCIO-1}]",
            "opt_design -quiet",
            "place_design -quiet",
            "route_design -quiet",
            "set refused [catch {write_bitstream -force %s.bit} err]" % name,
            expect,
        ],
    )
