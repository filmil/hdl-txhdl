# SPDX-License-Identifier: Apache-2.0
"""C++ compiled for Vreteno, and turned into an image the core runs.

The Rust side reaches the core through the ruleset's own toolchain and
a platform transition. C++ has no ruleset here, so the fetched
compiler is driven directly: one action compiles and links a
translation unit for this core, and a second turns the ELF into the
image, by the same tool and the same checks the Rust images go
through.
"""

# What this core is, in the compiler's words. `rv32imc` is the
# instruction set, compressed instructions included, `ilp32` the
# soft-float ABI, and both name the multilib of newlib and libstdc++
# the toolchain carries, so the standard library that gets linked is
# one built for this machine.
_ARCH = [
    "-march=rv32imc",
    "-mabi=ilp32",
]

# Why each of the rest. The instruction memory is 4 KiB, so size is
# not a preference. There is no unwinder and no operating system to
# unwind to, so exceptions are off; nothing reads a type at run time,
# so RTTI is off; and a static with a guard would call a lock that
# does not exist, so thread-safe statics are off. The entry point is
# the core's reset vector rather than a C runtime's, so the toolchain's
# own startup files are left out and the linker script says where
# everything goes.
_FLAGS = [
    "-Os",
    "-ffreestanding",
    "-fno-exceptions",
    "-fno-rtti",
    "-fno-threadsafe-statics",
    "-fno-use-cxa-atexit",
    "-ffunction-sections",
    "-fdata-sections",
    "-nostartfiles",
    "-static",
    "-Wall",
    "-Wextra",
    "-Werror",
    "-Wl,--gc-sections",
    "-Wl,--fatal-warnings",
]

def _vreteno_cc_image_impl(ctx):
    elf = ctx.actions.declare_file(ctx.label.name + ".elf")
    toolchain = ctx.attr._toolchain.files
    ctx.actions.run(
        inputs = depset(
            [ctx.file.src, ctx.file.linker_script],
            transitive = [toolchain],
        ),
        outputs = [elf],
        executable = ctx.file._gxx,
        arguments = _ARCH + _FLAGS + [
            "-T",
            ctx.file.linker_script.path,
            "-o",
            elf.path,
            ctx.file.src.path,
        ],
        mnemonic = "VretenoCxx",
        progress_message = "Compiling %s for Vreteno" % ctx.file.src.short_path,
    )
    out = ctx.actions.declare_file(ctx.label.name + ".rs")
    ctx.actions.run_shell(
        inputs = [elf],
        outputs = [out],
        tools = [ctx.executable._image],
        command = "'{}' '{}' > '{}'".format(
            ctx.executable._image.path,
            elf.path,
            out.path,
        ),
        mnemonic = "VretenoImage",
        progress_message = "Making a Vreteno image of %s" % elf.short_path,
    )
    return [DefaultInfo(files = depset([out]))]

vreteno_cc_image = rule(
    implementation = _vreteno_cc_image_impl,
    doc = "The Rust source of a Vreteno image, from one C++ file " +
          "compiled for the core.",
    attrs = {
        "src": attr.label(allow_single_file = [".cc", ".cpp"], mandatory = True),
        "linker_script": attr.label(allow_single_file = [".ld"], mandatory = True),
        "_toolchain": attr.label(default = "@riscv_none_elf_gcc//:all"),
        "_gxx": attr.label(
            default = "@riscv_none_elf_gcc//:gxx",
            allow_single_file = True,
        ),
        "_image": attr.label(
            default = "//tools/elf2vreteno",
            executable = True,
            cfg = "exec",
        ),
    },
)
