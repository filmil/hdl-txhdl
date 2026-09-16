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
