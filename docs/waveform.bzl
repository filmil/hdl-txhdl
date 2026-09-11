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
load("@rules_nvc//nvc:rules.bzl", "vhdl_test")
load("@rules_verilator//verilator:defs.bzl", "verilator_cc_library")
load("@rules_verilog//verilog:defs.bzl", "verilog_library")

def waveform(name, example, signals, lowered = None, until = None):
    """`lowered = (entity, unit)` also takes the example's VHDL and
    Verilog and simulates each against the trace: NAME.vhd, NAME.v and
    NAME.vhd.ports from the run, NAME_tb.vhd and NAME_tb.v from fst2tb,
    a vhdl_test NAME_sim under nvc and a cc_test NAME_vsim_test under
    Verilator. `until` cuts the figure at that tick, for a run too long
    to draw whole."""
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
        entity, unit = lowered
        native.genrule(
            name = name + "_tbgen",
            srcs = [name + ".fst", name + ".vhd.ports"],
            outs = [name + "_tb.vhd"],
            cmd = "$(location //tools/fst2tb) $(location " + name + ".fst)" +
                  " $(location " + name + ".vhd.ports) " + entity + " " + unit + " > $@",
            tools = ["//tools/fst2tb"],
        )
        vhdl_test(
            name = name + "_sim",
            srcs = [name + ".vhd", name + "_tb.vhd"],
            deps = [],
            entities = [entity + "_tb"],
        )
        native.genrule(
            name = name + "_tbgen_v",
            srcs = [name + ".fst", name + ".vhd.ports"],
            outs = [name + "_tb.v"],
            cmd = "$(location //tools/fst2tb) $(location " + name + ".fst)" +
                  " $(location " + name + ".vhd.ports) " + entity + " " + unit +
                  " --verilog > $@",
            tools = ["//tools/fst2tb"],
        )
        verilog_library(
            name = name + "_vl",
            srcs = [name + ".v", name + "_tb.v"],
            top_module = entity + "_tb",
        )
        verilator_cc_library(
            name = name + "_verilated",
            module = ":" + name + "_vl",
            timing = True,
            vopts = ["--main", "-Wno-fatal"],
        )
        cc_test(
            name = name + "_vsim_test",
            srcs = ["//tools/vlcheck:main.cc"],
            deps = [":" + name + "_verilated"],
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
