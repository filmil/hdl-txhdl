# SPDX-License-Identifier: Apache-2.0
"""A Rust test run as its tree builds optimised, its checks kept on.

A test that simulates a whole board spends its time in the simulation,
and the build's default for Rust is opt-level 0, at which the board ran
some 1500 cycles a second and `//cpu/vreteno:board_test` took 670 to
1100 s (issue 1250). `opt_test` builds a `rust_test` and everything it
links at opt-level 3, with debug assertions and overflow checks on as
they are at opt-level 0, so the test checks what it checked and runs
in a tenth of the time. The rest of the build, the same libraries'
other users included, keeps its defaults; the libraries are compiled
a second time for this configuration.

What is built for the core is not affected: the transition to the core
sets its own flags (issue 1248).
"""

OPT_RUSTC_FLAGS = [
    "-Copt-level=3",
    "-Cdebug-assertions=yes",
    "-Coverflow-checks=yes",
]

def _to_opt(settings, attr):
    _ = (settings, attr)  # the incoming configuration is not read
    return {"@rules_rust//rust/settings:extra_rustc_flags": OPT_RUSTC_FLAGS}

_opt_transition = transition(
    implementation = _to_opt,
    inputs = [],
    outputs = ["@rules_rust//rust/settings:extra_rustc_flags"],
)

def _opt_test_impl(ctx):
    test = ctx.attr.test[0][DefaultInfo]
    exe = test.files_to_run.executable
    out = ctx.actions.declare_file(ctx.label.name)
    ctx.actions.symlink(output = out, target_file = exe, is_executable = True)
    runfiles = ctx.runfiles(files = [exe]).merge(test.default_runfiles)
    return [DefaultInfo(executable = out, runfiles = runfiles)]

opt_test = rule(
    implementation = _opt_test_impl,
    doc = "Runs `test`, a `rust_test`, built optimised with its checks " +
          "on. Tag `test` manual, so that it is not also run unoptimised.",
    test = True,
    attrs = {
        "test": attr.label(
            doc = "The `rust_test` to build optimised and run.",
            cfg = _opt_transition,
            executable = True,
            mandatory = True,
        ),
    },
)
