# SPDX-License-Identifier: Apache-2.0
"""Building a program for Vreteno from a build for this machine.

A program for the core is compiled for another machine than the one
the build runs on, so the target that produces it has to be built
under another platform. A transition does that: `vreteno_image` takes
a `rust_binary`, builds it for the core, runs `//tools/elf2vreteno`
over the ELF, and gives back the Rust source of the image. Everything
else in the tree then depends on that source and nothing has to be
built by hand with a flag.
"""

def _to_vreteno(settings, attr):
    _ = settings  # the incoming configuration is not read
    return {"//command_line_option:platforms": str(attr.platform)}

_vreteno_transition = transition(
    implementation = _to_vreteno,
    inputs = [],
    outputs = ["//command_line_option:platforms"],
)

def _vreteno_image_impl(ctx):
    elf = ctx.executable.program
    out = ctx.actions.declare_file(ctx.label.name + ".rs")
    ctx.actions.run_shell(
        inputs = [elf],
        outputs = [out],
        tools = [ctx.executable._tool],
        command = "'{}' '{}' > '{}'".format(
            ctx.executable._tool.path,
            elf.path,
            out.path,
        ),
        mnemonic = "VretenoImage",
        progress_message = "Making a Vreteno image of %s" % elf.short_path,
    )
    return [DefaultInfo(files = depset([out]))]

vreteno_image = rule(
    implementation = _vreteno_image_impl,
    doc = "The Rust source of a Vreteno image, from a program built " +
          "for the core.",
    attrs = {
        "program": attr.label(
            doc = "The `rust_binary` to build for the core.",
            executable = True,
            cfg = _vreteno_transition,
            mandatory = True,
        ),
        "platform": attr.label(
            doc = "The platform the core is.",
            mandatory = True,
        ),
        "_tool": attr.label(
            default = "//tools/elf2vreteno",
            executable = True,
            cfg = "exec",
        ),
    },
)

def _vreteno_flat_impl(ctx):
    """The flat image of a program built for the core.

    A program that is loaded rather than built into the netlist goes to
    the board as bytes, so it wants the linked ELF flattened. The
    transition is the same one `vreteno_image` uses: without it the
    program is built for this machine, where its inline assembly is not
    even the right architecture.
    """
    elf = ctx.executable.program
    out = ctx.actions.declare_file(ctx.label.name + ".bin")
    ctx.actions.run(
        inputs = [elf],
        outputs = [out],
        executable = ctx.executable._objcopy,
        arguments = ["-O", "binary", elf.path, out.path],
        mnemonic = "VretenoFlat",
        progress_message = "Flattening %s for the board" % elf.short_path,
    )
    return [DefaultInfo(files = depset([out]))]

vreteno_flat = rule(
    implementation = _vreteno_flat_impl,
    doc = "The flat image of a program built for the core, for the " +
          "loader to take off the serial port.",
    attrs = {
        "program": attr.label(
            doc = "The `rust_binary` to build for the core.",
            executable = True,
            cfg = _vreteno_transition,
            mandatory = True,
        ),
        "platform": attr.label(
            doc = "The platform the core is.",
            mandatory = True,
        ),
        "_objcopy": attr.label(
            default = "@riscv_none_elf_gcc//:objcopy",
            executable = True,
            allow_single_file = True,
            cfg = "exec",
        ),
    },
)
