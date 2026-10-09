# SPDX-License-Identifier: Apache-2.0
"""Rules that build and run the RISC-V architectural tests (issue 1442).

They came from vreteno-conformance, which runs the same tests on the
core's Verilog netlist every day; here they run on the core's cycle
simulation, under `bazel test`, so that a conformance regression fails
a test in this tree.

The ACT4 framework turns one test source into one self-checking ELF in
four steps, in `framework/src/act/build_plan.py`. `act_elf` repeats them
as Bazel actions:

1. compile the test with `-DSIGNATURE`, which makes it write its
   results into a signature region;
2. run that ELF on the Sail reference model, which writes the
   signature to a file;
3. format the signature as assembler source, with `//tools/sigfmt`;
4. compile the test again with `-DRVTEST_SELFCHECK`, including that
   source, so the test compares every result it computes with the
   one Sail computed, and says PASS or FAIL itself.

`act_test` runs the final ELF on the core, with `//cpu/vreteno:act_run`,
and passes when the test says PASS and the run kept to the rules that
runner checks. The run is the test's and not a build action, so
`bazel build //...` builds every ELF and runs none.

`act_suite` makes these targets for every test in the catalog that
applies to the core, at each placement, and selects tests the way the
ACT4 framework does in `framework/src/act/select_tests.py`.
"""

ActElfInfo = provider(
    doc = "What act_elf built for one test.",
    fields = {"elf": "File: the self-checking ELF."},
)

# The XLEN every rule here builds for.
_XLEN = 32

def _config_dir(ctx):
    return ctx.file.rvmodel_macros.dirname

def _env_dir(ctx):
    for f in ctx.files.env:
        if f.basename == "riscv_arch_test.h":
            return f.dirname
    fail("the test environment has no riscv_arch_test.h")

def _march_flags(march):
    # As `Toolchain.march_flags` does for GCC and an assembly test: the
    # driver gets the base ISA, and the assembler gets the full string.
    march = march.replace("${XLEN}", str(_XLEN))
    return ["-march=rv%di" % _XLEN, "-Xassembler", "-march=" + march]

def _compile(ctx, out, extra, extra_inputs, mnemonic):
    test = ctx.file.src
    args = ctx.actions.args()
    args.add("-Wl,--no-warn-rwx-segments")
    args.add("-I" + _config_dir(ctx))
    args.add("-T" + ctx.file.linker_script.path)
    args.add_all(["-O0", "-g", "-mcmodel=medany", "-nostdlib"])
    args.add("-I" + _env_dir(ctx))
    args.add("-I.")
    args.add("-o", out)
    args.add_all(_march_flags(ctx.attr.march))
    args.add("-mabi=ilp32")
    args.add_all(extra)
    args.add("-DTEST_FLEN=32")
    args.add("-DTEST_FILE=\"%s\"" % test.basename)
    args.add(test)
    ctx.actions.run(
        executable = ctx.file._gcc,
        arguments = [args],
        inputs = depset(
            [test, ctx.file.linker_script, ctx.file.rvmodel_macros, ctx.file.rvtest_config] +
            ctx.files.env + extra_inputs,
            transitive = [depset(ctx.files._gcc_all)],
        ),
        outputs = [out],
        mnemonic = mnemonic,
        progress_message = "%s %s" % (mnemonic, ctx.label.name),
    )


def _act_elf_impl(ctx):
    name = ctx.label.name
    final_elf = ctx.actions.declare_file(name + ".elf")
    outputs = [final_elf]
    if ctx.attr.needs_signature:
        sig_elf = ctx.actions.declare_file(name + ".sig.elf")
        sig = ctx.actions.declare_file(name + ".sig")
        sig_log = ctx.actions.declare_file(name + ".sig.log")
        results = ctx.actions.declare_file(name + ".results")
        _compile(
            ctx,
            sig_elf,
            [
                "-DSIGNATURE",
                # Sail's devices, which `sail_macros.h` reaches.
                "-DSAIL_CLINT_BASE_ADDRESS=0x%x" % ctx.attr.sail_clint_base,
                "-DSAIL_SIMPLE_INTERRUPT_GENERATOR_BASE_ADDRESS=0x%x" % ctx.attr.sail_sig_base,
            ],
            [],
            "ActSigCompile",
        )
        ctx.actions.run_shell(
            command = """
"$1" --config "$2" --test-signature="$3" --signature-granularity 4 "$4" > "$5" 2>&1 || {
  echo "Sail failed on $4; its log follows." >&2
  while IFS= read -r line; do echo "$line" >&2; done < "$5"
  exit 1
}
""",
            arguments = [
                ctx.file._sail.path,
                ctx.file.sail_config.path,
                sig.path,
                sig_elf.path,
                sig_log.path,
            ],
            inputs = [sig_elf, ctx.file.sail_config],
            tools = [ctx.file._sail],
            outputs = [sig, sig_log],
            mnemonic = "ActSail",
            progress_message = "Running Sail for the signature of %s" % name,
        )
        ctx.actions.run(
            executable = ctx.executable._sigfmt,
            arguments = [sig.path, results.path],
            inputs = [sig],
            outputs = [results],
            mnemonic = "ActSigFmt",
        )
        _compile(
            ctx,
            final_elf,
            [
                "-DRVTEST_SELFCHECK",
                "-DSIGNATURE_FILE=\"%s\"" % results.path,
                "-DXLEN=%d" % _XLEN,
            ],
            [results],
            "ActCompile",
        )
        outputs += [sig, results]
    else:
        _compile(
            ctx,
            final_elf,
            ["-DRVTEST_SELFCHECK", "-DRVTEST_NOSIG", "-DXLEN=%d" % _XLEN],
            [],
            "ActCompile",
        )
    objdump = ctx.actions.declare_file(name + ".elf.objdump")
    ctx.actions.run_shell(
        command = "\"$1\" -d -M no-aliases,numeric \"$2\" > \"$3\"",
        arguments = [ctx.file._objdump.path, final_elf.path, objdump.path],
        inputs = [final_elf],
        tools = [ctx.file._objdump],
        outputs = [objdump],
        mnemonic = "ActObjdump",
    )
    return [
        ActElfInfo(elf = final_elf),
        DefaultInfo(files = depset([final_elf])),
        OutputGroupInfo(debug = depset(outputs + [objdump])),
    ]

act_elf = rule(
    implementation = _act_elf_impl,
    doc = "Builds one architectural test as a self-checking ELF.",
    attrs = {
        "src": attr.label(
            allow_single_file = [".S"],
            mandatory = True,
            doc = "The test source.",
        ),
        "march": attr.string(
            mandatory = True,
            doc = "The test's MARCH, from its header.",
        ),
        "needs_signature": attr.bool(
            default = True,
            doc = "Whether the test checks results computed by Sail.",
        ),
        "env": attr.label(
            default = "@riscv_arch_test//:env",
            doc = "The shared test environment headers.",
        ),
        "rvmodel_macros": attr.label(
            allow_single_file = True,
            default = "//cpu/vreteno/act/config:rvmodel_macros.h",
            doc = "The device's macros. Its directory is on the include path.",
        ),
        "rvtest_config": attr.label(
            allow_single_file = True,
            default = "//cpu/vreteno/act/config:rvtest_config.h",
            doc = "The device's configuration, beside rvmodel_macros.h.",
        ),
        "linker_script": attr.label(
            allow_single_file = True,
            default = "//cpu/vreteno/act/config:link.ld",
            doc = "The placement's memory map.",
        ),
        "sail_config": attr.label(
            allow_single_file = True,
            default = "//cpu/vreteno/act/config:sail.json",
            doc = "The Sail configuration that matches the device.",
        ),
        "sail_clint_base": attr.int(
            default = 0x2000000,
            doc = "platform.clint.base in sail_config.",
        ),
        "sail_sig_base": attr.int(
            default = 0xc000000,
            doc = "platform.simple_interrupt_generator.base in sail_config.",
        ),
        "_gcc": attr.label(
            allow_single_file = True,
            default = "@riscv_gcc15//:gcc",
        ),
        "_gcc_all": attr.label(
            default = "@riscv_gcc15//:all",
        ),
        "_objdump": attr.label(
            allow_single_file = True,
            default = "@riscv_gcc15//:objdump",
        ),
        "_sail": attr.label(
            allow_single_file = True,
            default = "@sail_riscv//:sail",
        ),
        "_sigfmt": attr.label(
            executable = True,
            cfg = "exec",
            default = "//tools/sigfmt",
        ),
    },
)

def _act_test_impl(ctx):
    elf = ctx.file.elf
    runner = ctx.executable._runner
    script = ctx.actions.declare_file(ctx.label.name + ".sh")
    ctx.actions.write(
        output = script,
        is_executable = True,
        content = """#!/usr/bin/env bash
# Runs {name} on the core and checks the verdict (issue 1442).
set -u
exec "{runner}" --elf "{elf}" --name "{name}" --max-cycles {cycles} --expect {expect}{excl}
""".format(
            runner = runner.short_path,
            elf = elf.short_path,
            name = ctx.attr.test_name,
            cycles = ctx.attr.max_cycles,
            expect = ctx.attr.expect,
            excl = " --excl" if ctx.attr.excl else "",
        ),
    )
    return [DefaultInfo(
        executable = script,
        runfiles = ctx.runfiles(files = [elf]).merge(
            ctx.attr._runner[DefaultInfo].default_runfiles,
        ),
    )]

act_test = rule(
    implementation = _act_test_impl,
    test = True,
    doc = "Runs one self-checking test ELF on the core's cycle simulation. " +
          "It passes when the verdict is the one expected.",
    attrs = {
        "elf": attr.label(
            allow_single_file = True,
            mandatory = True,
            doc = "The self-checking ELF, from act_elf.",
        ),
        "test_name": attr.string(
            mandatory = True,
            doc = "The test's name, for the runner's output.",
        ),
        "expect": attr.string(
            default = "pass",
            values = ["pass", "fail"],
            doc = "`fail` for a known failure: the test then fails once " +
                  "the run passes, so the entry is removed with the fix.",
        ),
        "max_cycles": attr.int(
            default = 20000000,
            doc = "The cycles after which a run is a timeout.",
        ),
        "excl": attr.bool(
            default = False,
            doc = "Run on the board's hart, whose A instructions on the " +
                  "DDR3 are exclusive pairs, with the exclusive monitor " +
                  "behind it (issue 1408).",
        ),
        "_runner": attr.label(
            executable = True,
            cfg = "target",
            default = "//cpu/vreteno:act_run",
        ),
    },
)

def _compare(test_value, config_value):
    """`_compare_param` from select_tests.py."""
    if type(test_value) == "string":
        for op in [">=", "<=", "!=", "==", ">", "<"]:
            if test_value.startswith(op):
                v = test_value[len(op):].strip()
                req = int(v, 16) if v.lower().startswith("0x") else int(v)
                if type(config_value) != "int":
                    return False
                return {
                    ">=": config_value >= req,
                    "<=": config_value <= req,
                    "!=": config_value != req,
                    "==": config_value == req,
                    ">": config_value > req,
                    "<": config_value < req,
                }[op]
    return test_value == config_value

def select_tests(tests, extensions, params, exclude_suites = []):
    """Selects the tests that apply to a device.

    Args:
      tests: the catalog, `TESTS` from `@riscv_arch_test//:tests.bzl`.
      extensions: the extensions the device implements, as UDB names
        them.
      params: the device's parameters, as a dict from the names the
        tests' headers use.
      exclude_suites: suites, by directory name, left out by hand.

    Returns:
      The selected entries of `tests`.
    """
    out = []
    for t in tests:
        if t["suite"] in exclude_suites or t["min_harts"] > 1:
            continue
        if [e for e in t["forbidden"] if e in extensions]:
            continue
        ok = True
        for r in t["required"]:
            if type(r) == "string":
                ok = ok and r in extensions
            else:
                ok = ok and len([e for e in r if e in extensions]) > 0
        for k, v in t["params"].items():
            ok = ok and k in params and _compare(v, params[k])
        if ok:
            out.append(t)
    return out

# Where a suite's tests are linked, by name: the linker script, and the
# word that ends each target's name.
PLACEMENTS = {
    # DDR3's range: fetches through the instruction cache, loads and
    # stores through the data cache.
    "cached": "//cpu/vreteno/act/config:link.ld",
    # Outside every range the core caches: every access a bus access.
    "uncached": "//cpu/vreteno/act/config:link_uncached.ld",
}

def act_suite(
        name,
        tests,
        known_failures = {},
        placements = PLACEMENTS.keys(),
        excl_suites = []):
    """Makes the targets for every test in `tests`, at each placement.

    For each test `T` and placement `P`: `T_P` is the self-checking
    ELF and `T_P_test` runs it on the core. `<name>_P_tests` is a test
    suite of a placement's tests, and `<name>_tests` of all of them. A
    test of a suite in `excl_suites` runs again as `T_P_excl_test`, on
    the board's hart with the exclusive monitor behind it, and
    `<name>_excl_tests` is those (issue 1408).

    Args:
      name: the suite's name.
      tests: the tests, from `select_tests`.
      known_failures: tests the core is known to fail at a placement,
        `"T_P"` mapped to the URL of the issue that tracks it. Their
        test passes while the run fails, and fails once the run passes,
        so that the entry is removed with the fix.
      placements: the placements to build and run, from PLACEMENTS.
      excl_suites: the suites, by the name a test's header gives, to
        run again with exclusive pairs.
    """
    excl = [t for t in tests if t["suite"] in excl_suites]
    native.test_suite(
        name = "%s_excl_tests" % name,
        tests = [
            "%s_%s_excl_test" % (t["name"], p)
            for t in excl
            for p in placements
        ],
    )
    for p in placements:
        native.test_suite(
            name = "%s_%s_tests" % (name, p),
            tests = ["%s_%s_test" % (t["name"], p) for t in tests],
        )
        for t in tests:
            act_elf(
                name = "%s_%s" % (t["name"], p),
                src = "@riscv_arch_test//:" + t["src"],
                march = t["march"],
                needs_signature = t["needs_signature"],
                linker_script = PLACEMENTS[p],
            )
            act_test(
                name = "%s_%s_test" % (t["name"], p),
                # Medium, not small: the slowest, Zalrsc-sc.w-00 from the DDR3
                # uncached, take about 15 s alone and ran past 60 s at a load
                # of 35, when every run shares the machine (issue 1408).
                size = "medium",
                elf = ":%s_%s" % (t["name"], p),
                test_name = "%s_%s" % (t["name"], p),
                expect = "fail" if "%s_%s" % (t["name"], p) in known_failures else "pass",
            )
            if t in excl:
                act_test(
                    name = "%s_%s_excl_test" % (t["name"], p),
                    size = "medium",
                    elf = ":%s_%s" % (t["name"], p),
                    test_name = "%s_%s_excl" % (t["name"], p),
                    excl = True,
                )
    native.test_suite(
        name = "%s_tests" % name,
        tests = ["%s_%s_tests" % (name, p) for p in placements],
    )
