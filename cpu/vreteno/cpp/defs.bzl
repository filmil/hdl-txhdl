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
# one built for this machine. `zicsr` is the control registers, which
# this assembler wants named before it will assemble a `csrwi`, and
# a program ends by writing one of them; it is an extension of the
# instruction set and not of the multilib, so the same libraries are
# still the ones linked.
_ARCH = [
    "-march=rv32imc_zicsr",
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

# A C program of many files, loaded rather than built into the boot
# memory (issue 1177): what `-Os` and the sections are for above, less
# the C++ ones. `-ffreestanding` is left out, since the program has the
# standard library, newlib's, as the toolchain carries it; the program
# gives newlib its system calls.
_C_FLAGS = [
    "-Os",
    "-ffunction-sections",
    "-fdata-sections",
]

def _vreteno_c_program_impl(ctx):
    toolchain = ctx.attr._toolchain.files
    gcc = ctx.file._gcc
    hdrs = depset(transitive = [h.files for h in ctx.attr.hdrs])
    dirs = {}
    for h in hdrs.to_list():
        dirs[h.dirname] = True
    # The roots headers are included from, as `<GLES/gl.h>` is from the
    # Khronos headers' package (#999): each file's package, under its root.
    for f in ctx.files.include_roots:
        root = f.root.path + "/" if f.root.path else ""
        dirs[root + f.owner.workspace_root + f.owner.package] = True
    hdrs = depset(ctx.files.include_roots, transitive = [hdrs])
    includes = ["-I" + d for d in dirs.keys()]
    objects = []
    for src in ctx.files.srcs:
        if src.extension == "h":
            continue
        obj = ctx.actions.declare_file(
            "_objs/{}/{}.o".format(ctx.label.name, src.short_path.replace("/", "_")),
        )
        ctx.actions.run(
            inputs = depset([src], transitive = [hdrs, toolchain]),
            outputs = [obj],
            executable = gcc,
            arguments = _ARCH + _C_FLAGS + ctx.attr.copts + includes + [
                "-c",
                src.path,
                "-o",
                obj.path,
            ],
            mnemonic = "VretenoCc",
            progress_message = "Compiling %s for Vreteno" % src.short_path,
        )
        objects.append(obj)
    elf = ctx.actions.declare_file(ctx.label.name + ".elf")
    ctx.actions.run(
        inputs = depset(
            objects + ctx.files.libs + [ctx.file.linker_script],
            transitive = [toolchain],
        ),
        outputs = [elf],
        executable = gcc,
        arguments = _ARCH + [
            "-nostartfiles",
            "-static",
            "-specs=nano.specs",
            "-Wl,--gc-sections",
            "-T",
            ctx.file.linker_script.path,
            "-o",
            elf.path,
        ] + [o.path for o in objects] + [l.path for l in ctx.files.libs] +
        ["-lc", "-lm", "-lgcc"],
        mnemonic = "VretenoLink",
        progress_message = "Linking %s for Vreteno" % ctx.label.name,
    )
    flat = ctx.actions.declare_file(ctx.label.name + ".bin")
    ctx.actions.run(
        inputs = [elf],
        outputs = [flat],
        executable = ctx.file._objcopy,
        arguments = ["-O", "binary", elf.path, flat.path],
        mnemonic = "VretenoFlat",
        progress_message = "Making the flat image of %s" % ctx.label.name,
    )
    return [DefaultInfo(files = depset([flat, elf]))]

vreteno_c_program = rule(
    implementation = _vreteno_c_program_impl,
    doc = "A C program of many files compiled for the core with newlib, " +
          "linked as the linker script says and made a flat image, " +
          "`<name>.bin`, which the serial loader or fastboot's boot runs, " +
          "beside its ELF, `<name>.elf`.",
    attrs = {
        "srcs": attr.label_list(allow_files = [".c", ".S", ".h"], mandatory = True),
        "hdrs": attr.label_list(allow_files = [".h"]),
        "copts": attr.string_list(),
        "include_roots": attr.label_list(
            allow_files = True,
            doc = "Headers included by their path from their package, " +
                  "whose package's directory is put on the include path.",
        ),
        "libs": attr.label_list(
            allow_files = [".a"],
            doc = "Static libraries for the core, linked after the objects.",
        ),
        "linker_script": attr.label(allow_single_file = [".ld"], mandatory = True),
        "_toolchain": attr.label(default = "@riscv_none_elf_gcc//:all"),
        "_gcc": attr.label(
            default = "@riscv_none_elf_gcc//:gcc",
            allow_single_file = True,
        ),
        "_objcopy": attr.label(
            default = "@riscv_none_elf_gcc//:objcopy",
            allow_single_file = True,
        ),
    },
)
