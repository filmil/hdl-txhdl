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

def waveform(name, example, signals):
    native.genrule(
        name = name + "_run",
        outs = ["out_" + name + ".txt", name + ".fst", name + ".fst.names"],
        cmd = "TXHDL_FST=$(RULEDIR)/" + name + ".fst $(location " + example + ")" +
              " > $(RULEDIR)/out_" + name + ".txt",
        tools = [example],
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
    native.genrule(
        name = name + "_timing",
        srcs = [name + ".dt", name + ".fst.names"],
        outs = [name + "_timing.tex"],
        cmd = "$(location //tools/dt2tikz) $(location " + name + ".dt)" +
              " --order " + order + " --color" +
              " --names $(location " + name + ".fst.names)" +
              " --signals '" + ",".join(signals) + "' > $@",
        tools = ["//tools/dt2tikz"],
    )
