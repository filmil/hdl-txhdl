# SPDX-License-Identifier: Apache-2.0
"""An example's waveform, drawn by the build.

Runs the example with `TXHDL_FST` set, so it prints as usual and writes
its FST as well; turns the FST into a signals database (vcdcvt, which
reads FST by the file's extension) and the
chosen signals into drawtiming text (sqlite2drawtiming), both prebuilt
tools pinned in //:multitool.lock.json; and draws the text as TikZ
(//tools/dt2tikz), in colour. Produces out_NAME.txt, NAME.dt and
NAME_timing.tex.
"""

load("@rules_cc//cc:cc_test.bzl", "cc_test")
load("@rules_shell//shell:sh_test.bzl", "sh_test")
load("@rules_nvc//nvc:rules.bzl", "vhdl_test")
load("@rules_verilator//verilator:defs.bzl", "verilator_cc_library")
load("@rules_verilog//verilog:defs.bzl", "verilog_library")

def waveform(
        name,
        example,
        signals,
        lowered = None,
        until = None,
        from_tick = None,
        width = None,
        foreign_vhdl = [],
        foreign_verilog = []):
    """`lowered = (entity, unit)` also takes the example's VHDL and
    Verilog and simulates each against the trace: NAME.vhd, NAME.v and
    NAME.vhd.ports from the run, NAME_tb.vhd and NAME_tb.v from fst2tb,
    a vhdl_test NAME_sim under nvc and a cc_test NAME_vsim_test under
    Verilator. A list of pairs checks several units of the one run,
    the targets then named NAME_sim_ENTITY and NAME_vsim_ENTITY_test.
    `until` cuts the figure at that tick, for a run too long to draw
    whole, and `from_tick` starts it at one, a window on one thing the
    run does; `width` is the figure's width in centimetres.
    `foreign_vhdl` and `foreign_verilog` are the sources of the foreign
    modules a lowered unit instantiates, simulated with its netlist in
    that language, since the netlist names them and does not write
    them."""
    outs = ["out_" + name + ".txt", name + ".fst", name + ".fst.names"]
    env = "TXHDL_FST=$(RULEDIR)/" + name + ".fst"
    if lowered:
        outs += [name + ".vhd", name + ".vhd.ports", name + ".v"]
        env += " TXHDL_VHDL=$(RULEDIR)/" + name + ".vhd"
        env += " TXHDL_VERILOG=$(RULEDIR)/" + name + ".v"
    native.genrule(
        name = name + "_run",
        outs = outs,
        cmd = env + " $(location " + example + ")" +
              " > $(RULEDIR)/out_" + name + ".txt",
        tools = [example],
    )
    if lowered:
        pairs = lowered if type(lowered) == "list" else [lowered]
        for entity, unit in pairs:
            # One lowered unit keeps the short names; several are told
            # apart by the entity.
            tag = "" if len(pairs) == 1 else "_" + entity
            tb = name + tag + "_tb"
            native.genrule(
                name = name + tag + "_tbgen",
                srcs = [name + ".fst", name + ".vhd.ports"],
                outs = [tb + ".vhd"],
                cmd = "$(location //tools/fst2tb) $(location " + name + ".fst)" +
                      " $(location " + name + ".vhd.ports) " + entity + " " + unit + " > $@",
                tools = ["//tools/fst2tb"],
            )
            vhdl_test(
                name = name + "_sim" + tag,
                srcs = foreign_vhdl + [name + ".vhd", tb + ".vhd"],
                deps = [],
                entities = [entity + "_tb"],
            )
            native.genrule(
                name = name + tag + "_tbgen_v",
                srcs = [name + ".fst", name + ".vhd.ports"],
                outs = [tb + ".v"],
                cmd = "$(location //tools/fst2tb) $(location " + name + ".fst)" +
                      " $(location " + name + ".vhd.ports) " + entity + " " + unit +
                      " --verilog > $@",
                tools = ["//tools/fst2tb"],
            )
            verilog_library(
                name = name + tag + "_vl",
                srcs = foreign_verilog + [name + ".v", tb + ".v"],
                top_module = entity + "_tb",
            )
            verilator_cc_library(
                name = name + tag + "_verilated",
                module = ":" + name + tag + "_vl",
                # Verilator's own headers, not the netlist (issue 536).
                copts = ["-Wno-sign-compare"],
                timing = True,
                vopts = ["--main", "-Wno-fatal"],
            )
            cc_test(
                name = name + "_vsim" + tag + "_test",
                srcs = ["//tools/vlcheck:main.cc"],
                deps = [":" + name + tag + "_verilated"],
            )
    native.genrule(
        name = name + "_db",
        srcs = [name + ".fst"],
        outs = [name + ".db"],
        cmd = "$(location @multitool//tools/vcdcvt) -logtostderr" +
              " -in $(location " + name + ".fst) -format sqlite -out $@",
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
    if from_tick:
        cut += " --from %d" % from_tick
    if width:
        cut += " --width %s" % width
    native.genrule(
        name = name + "_timing",
        srcs = [name + ".dt", name + ".fst.names"],
        outs = [name + "_timing.tex"],
        cmd = "$(location //tools/dt2tikz) $(location " + name + ".dt)" +
              " --order " + order + " --color" + cut +
              " --names $(location " + name + ".fst.names)" +
              " --signals '" + ",".join(signals) + "' > $@",
        tools = ["//tools/dt2tikz"],
    )
    # The diagram fits every column it is put in, or the test says
    # which placement to make figure* (issue 324). The check reads the
    # documents' sources, so a placement added later is caught too.
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
