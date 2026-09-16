# SPDX-License-Identifier: Apache-2.0
"""The two steps of an ASIC flow, as Bazel rules.

`asic_synth` maps Verilog onto a standard cell library with Yosys, and
`asic_pnr` floorplans, places, routes and times the result with
OpenROAD. Both tools are unpacked Debian trees fetched by checksum
(see //third_party/debs), so neither rule uses anything installed on
the machine; each finds the root of its tool's tree from the path of
one file in it and runs the binary there with its own libraries.
"""

def _tree_root(files, suffix):
    """The execroot path of the tree that holds a file ending in `suffix`."""
    for f in files:
        if f.path.endswith(suffix):
            return f.path[:-len(suffix)]
    fail("no file ending in %s among %d files" % (suffix, len(files)))

def _asic_synth_impl(ctx):
    yosys = ctx.attr.yosys[DefaultInfo].files.to_list()
    ytree = _tree_root(yosys, "/usr/bin/yosys")
    platform = ctx.attr.platform[DefaultInfo].files.to_list()
    proot = _tree_root(platform, "/setRC.tcl")

    netlist = ctx.actions.declare_file(ctx.label.name + ".v")
    log = ctx.actions.declare_file(ctx.label.name + ".log")
    stats = ctx.actions.declare_file(ctx.label.name + ".stat")

    script = ctx.actions.declare_file(ctx.label.name + ".ys")
    ctx.actions.expand_template(
        template = ctx.file.script,
        output = script,
        substitutions = {
            "@SOURCES@": " ".join([s.path for s in ctx.files.srcs]),
            "@TOP@": ctx.attr.top,
            "@LIBERTY@": proot + "/" + ctx.attr.liberty,
            "@PERIOD_PS@": str(ctx.attr.period_ps),
            "@NETLIST@": netlist.path,
            "@STATS@": stats.path,
        },
    )

    ctx.actions.run_shell(
        inputs = depset(
            ctx.files.srcs + [script],
            transitive = [
                ctx.attr.yosys[DefaultInfo].files,
                ctx.attr.platform[DefaultInfo].files,
            ],
        ),
        outputs = [netlist, log, stats],
        command = _SYNTH_COMMAND,
        arguments = [ytree, script.path, log.path],
        mnemonic = "AsicSynth",
        progress_message = "Mapping %s onto standard cells" % ctx.attr.top,
        use_default_shell_env = False,
        env = {"HOME": "/tmp"},
    )

    return [DefaultInfo(files = depset([netlist, log, stats]))]

_SYNTH_COMMAND = """
set -eu
tree="$PWD/$1"
export LD_LIBRARY_PATH="$tree/usr/lib/x86_64-linux-gnu:$tree/lib/x86_64-linux-gnu"
# Yosys reaches for its own share directory beside the binary and for
# the ABC it was built with, both of which the package puts where the
# binary expects them.
exec "$tree/usr/bin/yosys" -l "$3" -s "$2"
"""

asic_synth = rule(
    implementation = _asic_synth_impl,
    doc = "Maps Verilog onto a standard cell library with Yosys.",
    attrs = {
        "srcs": attr.label_list(
            allow_files = [".v"],
            mandatory = True,
            doc = "The Verilog to synthesise.",
        ),
        "top": attr.string(mandatory = True, doc = "The top module."),
        "script": attr.label(
            allow_single_file = True,
            mandatory = True,
            doc = "The Yosys script, with @SOURCES@ and the rest to fill in.",
        ),
        "liberty": attr.string(
            mandatory = True,
            doc = "The timing library, as a path inside the platform tree.",
        ),
        "period_ps": attr.int(
            default = 3000,
            doc = "The clock period the technology mapper aims at, in ps.",
        ),
        "yosys": attr.label(default = "//third_party/yosys:tree"),
        "platform": attr.label(default = "//third_party/nangate45:tree"),
    },
)

def _asic_pnr_impl(ctx):
    openroad = ctx.attr.openroad[DefaultInfo].files.to_list()
    otree = _tree_root(openroad, "/usr/bin/openroad")
    ortools = ctx.attr.ortools[DefaultInfo].files.to_list()
    orlib = _tree_root(ortools, "/libortools.so.9")
    platform = ctx.attr.platform[DefaultInfo].files.to_list()
    proot = _tree_root(platform, "/setRC.tcl")

    # `asic_synth` yields a netlist, a log and a table of cell counts;
    # this step wants the netlist.
    netlist = None
    for f in ctx.attr.netlist[DefaultInfo].files.to_list():
        if f.extension == "v":
            netlist = f
    if netlist == None:
        fail("%s produced no Verilog" % ctx.attr.netlist.label)

    outs = {}
    for key, suffix in _PNR_OUTPUTS.items():
        outs[key] = ctx.actions.declare_file(ctx.label.name + suffix)

    ctx.actions.run_shell(
        inputs = depset(
            [netlist, ctx.file.sdc, ctx.file.script],
            transitive = [
                ctx.attr.openroad[DefaultInfo].files,
                ctx.attr.ortools[DefaultInfo].files,
                ctx.attr.platform[DefaultInfo].files,
            ],
        ),
        outputs = outs.values(),
        command = _PNR_COMMAND,
        arguments = [otree, orlib, ctx.file.script.path, outs["log"].path],
        mnemonic = "AsicPnr",
        progress_message = "Placing and routing %s" % ctx.attr.top,
        use_default_shell_env = False,
        env = {
            "HOME": "/tmp",
            "TXHDL_PLATFORM": proot,
            "TXHDL_NETLIST": netlist.path,
            "TXHDL_SDC": ctx.file.sdc.path,
            "TXHDL_TOP": ctx.attr.top,
            "TXHDL_THREADS": str(ctx.attr.threads),
            "TXHDL_UTILIZATION": str(ctx.attr.utilization),
            "TXHDL_DENSITY": str(ctx.attr.density),
            "TXHDL_DEF": outs["def"].path,
            "TXHDL_ROUTED": outs["routed"].path,
            "TXHDL_DRC": outs["drc"].path,
            "TXHDL_METRICS": outs["metrics"].path,
            "TXHDL_MAP": outs["map"].path,
        },
    )

    return [DefaultInfo(files = depset(outs.values()))]

_PNR_OUTPUTS = {
    "log": ".log",
    "def": ".def",
    "routed": "_routed.v",
    "drc": "_drc.rpt",
    "metrics": ".tsv",
    "map": "_map.tsv",
}

_PNR_COMMAND = """
set -eu
tree="$PWD/$1"
export LD_LIBRARY_PATH="$tree/usr/lib/x86_64-linux-gnu:$tree/lib/x86_64-linux-gnu:$PWD/$2"
export TCL_LIBRARY="$tree/usr/share/tcltk/tcl8.6"
# No window is ever opened, and the package links against Qt whether or
# not one is: telling Qt to draw into nothing keeps it from looking for
# a display that a build machine does not have.
export QT_QPA_PLATFORM=offscreen
exec "$tree/usr/bin/openroad" -no_init -exit -log "$4" "$3"
"""

asic_pnr = rule(
    implementation = _asic_pnr_impl,
    doc = "Floorplans, places, routes and times a netlist with OpenROAD.",
    attrs = {
        "netlist": attr.label(
            mandatory = True,
            doc = "The asic_synth target whose netlist is to be laid out.",
        ),
        "sdc": attr.label(
            allow_single_file = [".sdc"],
            mandatory = True,
            doc = "The constraints: the clock, and the delays at the ports.",
        ),
        "script": attr.label(
            allow_single_file = [".tcl"],
            mandatory = True,
            doc = "The OpenROAD script; it reads the rest from the environment.",
        ),
        "top": attr.string(mandatory = True),
        "utilization": attr.int(
            default = 30,
            doc = "The percentage of the core area the cells are to fill.",
        ),
        "density": attr.string(
            default = "0.60",
            doc = "The density the global placer spreads the cells to.",
        ),
        "threads": attr.int(
            default = 8,
            doc = """How many threads the router may use.

            Pin access and routing are parallel and OpenROAD's default
            is one thread, which turns twenty minutes of work into
            three hours. It is a number rather than the machine's own
            count because an action must not read the machine.
            """,
        ),
        "openroad": attr.label(default = "//third_party/openroad:tree"),
        "ortools": attr.label(default = "//third_party/openroad:ortools"),
        "platform": attr.label(default = "//third_party/nangate45:tree"),
    },
)
