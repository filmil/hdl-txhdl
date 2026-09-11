# SPDX-License-Identifier: Apache-2.0
"""An example's waveform, drawn by the build.

Runs the example with `TXHDL_VCD` set, so it prints as usual and writes
its VCD as well; turns the VCD into a signals database (vcdcvt) and the
chosen signals into drawtiming text (sqlite2drawtiming), both prebuilt
tools pinned in //:multitool.lock.json; and draws the text as TikZ
(//tools/dt2tikz). Produces out_NAME.txt, NAME.dt and NAME_timing.tex.
"""

def waveform(name, example, signals):
    native.genrule(
        name = name + "_run",
        outs = ["out_" + name + ".txt", name + ".vcd"],
        cmd = "TXHDL_VCD=$(RULEDIR)/" + name + ".vcd $(location " + example + ")" +
              " > $(RULEDIR)/out_" + name + ".txt",
        tools = [example],
    )
    native.genrule(
        name = name + "_db",
        srcs = [name + ".vcd"],
        outs = [name + ".db"],
        cmd = "$(location @multitool//tools/vcdcvt) -logtostderr" +
              " -in $(location " + name + ".vcd) -format sqlite -out $@",
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
    native.genrule(
        name = name + "_timing",
        srcs = [name + ".dt"],
        outs = [name + "_timing.tex"],
        cmd = "$(location //tools/dt2tikz) $(location " + name + ".dt)" +
              " --order " + order + " > $@",
        tools = ["//tools/dt2tikz"],
    )
