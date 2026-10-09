# SPDX-License-Identifier: Apache-2.0
"""A test run under a size its own rule does not let it be given.

rules_nvc's `vhdl_test` takes no `size`, so every replay it makes is
medium, with 300 s, and a long one times out on a loaded host (issue
1448; filmil/bazel_rules_nvc issue 110). `sized_test` runs another
test's executable with that test's runfiles, as a test of its own,
which takes `size` as every test does. Tag the wrapped test manual, so
that it is not also run under its own size.
"""

def _sized_test_impl(ctx):
    test = ctx.attr.test[DefaultInfo]
    exe = test.files_to_run.executable
    out = ctx.actions.declare_file(ctx.label.name)
    ctx.actions.symlink(output = out, target_file = exe, is_executable = True)
    runfiles = ctx.runfiles(files = [exe]).merge(test.default_runfiles)
    return [DefaultInfo(executable = out, runfiles = runfiles)]

sized_test = rule(
    implementation = _sized_test_impl,
    doc = "Runs `test` as a test of its own, under this target's " +
          "`size`. Tag `test` manual.",
    test = True,
    attrs = {
        "test": attr.label(
            doc = "The test to run.",
            cfg = "target",
            executable = True,
            mandatory = True,
        ),
    },
)
