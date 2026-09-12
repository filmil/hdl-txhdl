# SPDX-License-Identifier: Apache-2.0
"""A foreign module as a unit: `verilog_unit()` and `vhdl_unit()`."""

load("@rules_cc//cc:cc_library.bzl", "cc_library")
load("@rules_nvc//internal:providers.bzl", "ElaborateProvider", "VHDLLibraryProvider")
load(
    "@rules_nvc//internal:toolchain.bzl",
    "NVC_TOOLCHAIN_TYPE",
    "NVC_WRAPPER",
    "VHDL_STANDARD_DEFAULT",
)
load("@rules_nvc//internal:utils.bzl", "get_nvc_ld_library_path", "get_single_file_from")
load("@rules_nvc//nvc:rules.bzl", "vhdl_elaborate", "vhdl_library")
load("@rules_rust//rust:defs.bzl", "rust_library")
load("@rules_verilator//verilator:defs.bzl", "verilator_cc_library")
load("@rules_verilog//verilog:defs.bzl", "verilog_library")

def verilog_unit(name, src, top, clock = "clk"):
    """A Verilog module as a unit, in a crate of its own.

    `src` is the Verilog file, `top` the module in it, `clock` its
    clock port. Verilator compiles the module into a C++ model;
    `//tools/vshim` writes a C shim over the model and the Rust that
    makes a `Unit` of it, named after the module in CamelCase, with an
    `In` or `Out` per port and an `Rx` or `Tx` per `x_data`, `x_valid`,
    `x_ready` trio; and the crate `name` holds that unit, for an
    example or a design to depend on."""
    verilog_library(
        name = name + "_vl",
        srcs = [src],
        top_module = top,
    )
    verilator_cc_library(
        name = name + "_verilated",
        module = ":" + name + "_vl",
        vopts = ["-Wno-fatal"],
    )
    native.genrule(
        name = name + "_gen",
        srcs = [src],
        outs = [name + "_shim.cc", name + ".rs"],
        cmd = "$(location //tools/vshim) $(location " + src + ") " + top +
              " " + clock + " " + name + " $(location " + name + "_shim.cc)" +
              " $(location " + name + ".rs)",
        tools = ["//tools/vshim"],
    )
    cc_library(
        name = name + "_shim",
        srcs = [name + "_shim.cc"],
        deps = [":" + name + "_verilated"],
    )
    rust_library(
        name = name,
        srcs = [name + ".rs"],
        crate_name = name,
        crate_root = name + ".rs",
        edition = "2021",
        deps = ["//lib:txhdl", ":" + name + "_shim"],
    )

def vhdl_unit(name, src, entity, clock = "clk"):
    """A VHDL entity as a unit, in a crate of its own, with nvc as a
    child process.

    `src` is the VHDL file, `entity` the entity in it, `clock` its
    clock port. `//tools/vshim` writes a testbench around the entity
    that takes a step per line on its standard input and answers with
    the outputs; `vhdl_test` analyses and elaborates the two and makes
    the script that runs nvc on them, tagged manual since it waits on
    its input; and the crate `name` holds the unit, which starts the
    script and speaks the lines. The unit's ports are as
    `verilog_unit()` makes them."""
    tb = name + "_cosim"
    native.genrule(
        name = name + "_tbgen",
        srcs = [src],
        outs = [tb + ".vhd"],
        cmd = "$(location //tools/vshim) --vhdl-tb $(location " + src +
              ") " + entity + " " + clock + " " + name + " $@",
        tools = ["//tools/vshim"],
    )
    vhdl_library(
        name = name + "_lib",
        srcs = [src, tb + ".vhd"],
        deps = [],
        standard = VHDL_STANDARD_DEFAULT,
    )
    vhdl_elaborate(
        name = tb,
        library = ":" + name + "_lib",
        standard = VHDL_STANDARD_DEFAULT,
    )
    script = name + "_run"
    nvc_runner(
        name = script,
        entity = ":" + tb,
    )
    rlocation = native.package_name() + "/" + script
    native.genrule(
        name = name + "_gen",
        srcs = [src],
        outs = [name + ".rs"],
        cmd = "$(location //tools/vshim) --vhdl-unit $(location " + src +
              ") " + entity + " " + clock + " " + name + " " + rlocation +
              " $@",
        tools = ["//tools/vshim"],
    )
    rust_library(
        name = name,
        srcs = [name + ".rs"],
        crate_name = name,
        crate_root = name + ".rs",
        edition = "2021",
        data = [":" + script],
        deps = ["//lib:txhdl"],
    )

# The script that runs nvc on an elaborated entity, as rules_nvc's test
# rule makes it but as a plain executable, so that a library may carry
# it as data: a test-only target could not be depended on.
def _nvc_runner(ctx):
    nvc_info = ctx.toolchains[NVC_TOOLCHAIN_TYPE].nvc_info
    analyzer_x = nvc_info.analyzer.files.to_list()[0]
    analyzer = analyzer_x.short_path
    artifacts = nvc_info.artifacts_dir.files.to_list()
    std_lib_dir = artifacts[0]
    analyzer_dir = analyzer_x.dirname
    base_dir = analyzer_dir[:-4] if analyzer_dir.endswith("/bin") else analyzer_dir
    nvc_lib_path = std_lib_dir.short_path
    vhdl_provider = ctx.attr.entity[VHDLLibraryProvider]
    flag_libraries = []
    deps_paths = []
    seen = []
    for lib_name, path in vhdl_provider.libraries:
        if lib_name != vhdl_provider.library_name and lib_name not in seen:
            flag_libraries += ["--map={}:{}/{}".format(lib_name, path.short_path, lib_name)]
            deps_paths += [path]
            seen += [lib_name]
    work_library_file = get_single_file_from(ctx.attr.entity)
    elaborate_provider = ctx.attr.entity[ElaborateProvider]
    runfiles = ctx.runfiles(
        files = [vhdl_provider.library_dir] + ctx.attr._script.files.to_list(),
        transitive_files = depset(artifacts + deps_paths),
    )
    runfiles = runfiles.merge(ctx.attr._script[DefaultInfo].default_runfiles)

    def for_wrapper(p):
        return ("../" + p[9:]) if p.startswith("external/") else p

    nvc_ld_library_path = get_nvc_ld_library_path(nvc_info, base_dir, ctx.configuration.default_shell_env)
    ld_path = ":".join([for_wrapper(p) for p in nvc_ld_library_path.split(":")])
    ctx.actions.expand_template(
        template = ctx.file._template,
        output = ctx.outputs.executable,
        substitutions = {
            "{{EXECUTABLE}}": "NVC_LD_LIBRARY_PATH=\"" + ld_path + "\" LD_LIBRARY_PATH=\"" +
                              for_wrapper(base_dir) + "/lib/x86_64-linux-gnu\" " +
                              ctx.executable._script.short_path,
            "{{VHDL_STANDARD}}": VHDL_STANDARD_DEFAULT,
            "{{ANALYZER}}": for_wrapper(analyzer),
            "{{LIBRARY_NAME}}": vhdl_provider.library_name,
            "{{LIBRARY_PATHS}}": " ".join(flag_libraries + ["-L", for_wrapper(nvc_lib_path)]),
            "{{STDLIB_DIR}}": for_wrapper(nvc_lib_path),
            "{{ENTITY}}": elaborate_provider.entity,
            "{{LIB_DIR_IN_PATH}}": vhdl_provider.library_dir.short_path,
            "{{LIB_DIR_OUT_PATH}}": work_library_file.short_path,
            "{{WAVE_FILE}}": "{}.fst".format(ctx.attr.name),
            "{{VPI_FLAGS}}": "",
            "{{EXTRA_ARGS}}": "",
        },
    )
    return [DefaultInfo(runfiles = runfiles)]

nvc_runner = rule(
    doc = "Runs nvc on an elaborated entity; an executable, not a test.",
    executable = True,
    implementation = _nvc_runner,
    attrs = {
        "entity": attr.label(doc = "The elaborated entity."),
        "_script": attr.label(
            default = NVC_WRAPPER,
            executable = True,
            cfg = "exec",
        ),
        "_template": attr.label(
            default = Label("@rules_nvc//build/nvc:unittest.tpl.sh"),
            allow_single_file = True,
        ),
    },
    toolchains = [NVC_TOOLCHAIN_TYPE],
)
