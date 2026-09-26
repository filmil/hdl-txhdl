# SPDX-License-Identifier: Apache-2.0
"""A unit's checks proved, searched or covered with SymbiYosys.

`formal_test` runs SymbiYosys on the Verilog a `waveform()` wrote, in
one of its three modes, and passes when the run ends the way it is
expected to: a proof that holds, or a broken unit that fails.
`formal_run` is the same run as a build step, for a document that
includes the run's log and its trace. Both run formal.sh, which says
how the tools are found and what the modes mean (issue 581).
"""

load("@rules_shell//shell:sh_test.bzl", "sh_test")

_TOOLS = [
    "//third_party/sby:sby",
    "@python_3_12_host//:python",
]

_TREES = [
    "//third_party/yosys:tree",
    "//third_party/z3:tree",
]

def _args(verilog, top, mode, depth, expect):
    return [
        "$(rootpath %s)" % verilog,
        top,
        mode,
        str(depth),
        str(expect),
        "$(rootpath //third_party/sby:sby)",
        "$(rootpath @python_3_12_host//:python)",
    ]

def formal_test(name, verilog, top, mode = "prove", depth = 20, expect = 0, **kwargs):
    """A SymbiYosys run that must end with the exit code `expect`.

    Args:
      name: the test's name.
      verilog: the Verilog file, such as a `waveform()`'s `NAME.v`.
      top: the module to prove, since the file holds every unit of
        the run.
      mode: `prove` for an unbounded proof by `abc pdr`, which needs no
        solver; `bmc` for a bounded search of `depth` steps with z3,
        which gives a counterexample; `cover` for a trace to each
        cover point.
      depth: the steps of a bounded run.
      expect: SymbiYosys's exit code, 0 for a pass and 2 for a
        failure; an UNKNOWN never passes.
      **kwargs: passed to sh_test.
    """
    sh_test(
        name = name,
        srcs = ["//tools/formal:formal.sh"],
        args = _args(verilog, top, mode, depth, expect),
        data = [verilog] + _TOOLS + _TREES,
        size = kwargs.pop("size", "small"),
        **kwargs
    )

def formal_run(name, verilog, top, mode = "prove", depth = 20, expect = 0, **kwargs):
    """The same run as a build step: `NAME.log` is the run's log with
    its timings removed, and `NAME.vcd` the trace it wrote, empty when
    a run leaves none."""
    native.genrule(
        name = name,
        srcs = [verilog] + _TREES,
        outs = [name + ".log", name + ".vcd"],
        cmd = " ".join(
            ["$(location //tools/formal:formal.sh)"] +
            [
                "$(location %s)" % verilog,
                top,
                mode,
                str(depth),
                str(expect),
                "$(location //third_party/sby:sby)",
                "$(location @python_3_12_host//:python)",
                "$(location %s.log)" % name,
                "$(location %s.vcd)" % name,
            ],
        ) + " > /dev/null",
        tools = ["//tools/formal:formal.sh"] + _TOOLS,
        **kwargs
    )

def formal_trace(name, run, signals, width = None, until = None):
    """The trace a `formal_run` wrote, drawn as a timing diagram,
    `NAME_timing.tex`, through the same three tools a `waveform()`
    uses on a run's FST; a counterexample or the path to a cover point
    is a picture rather than a file, and `NAME_width_test` says that
    it fits its column."""
    native.genrule(
        name = name + "_db",
        srcs = [run + ".vcd"],
        outs = [name + ".db"],
        cmd = "$(location @multitool//tools/vcdcvt) -logtostderr" +
              " -in $(location " + run + ".vcd) -format sqlite -out $@",
        tools = ["@multitool//tools/vcdcvt"],
    )
    native.genrule(
        name = name + "_dt",
        srcs = [name + ".db"],
        outs = [name + ".dt"],
        cmd = "$(location @multitool//tools/sqlite2drawtiming) -logtostderr" +
              " -in $(location " + name + ".db) -ndots 1 " +
              " ".join(["-signal '" + s + "'" for s in signals]) + " > $@",
        tools = ["@multitool//tools/sqlite2drawtiming"],
    )
    order = ",".join([s.split("=>")[-1] for s in signals])
    cut = " --until %d" % until if until else ""
    if width:
        cut += " --width %s" % width
    native.genrule(
        name = name + "_timing",
        srcs = [name + ".dt"],
        outs = [name + "_timing.tex"],
        cmd = "$(location //tools/dt2tikz) $(location " + name + ".dt)" +
              " --order " + order + " --color" + cut + " > $@",
        tools = ["//tools/dt2tikz"],
    )
    sh_test(
        name = name + "_width_test",
        srcs = ["//tools/figwidth:check.sh"],
        args = [
            "$(rootpath //tools/figwidth)",
            "$(rootpath " + name + "_timing.tex)",
            "$(rootpaths //docs:tex_sources)",
        ],
        data = [
            name + "_timing.tex",
            "//docs:tex_sources",
            "//tools/figwidth",
        ],
        size = "small",
    )
