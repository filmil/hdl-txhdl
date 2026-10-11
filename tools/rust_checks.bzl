# SPDX-License-Identifier: Apache-2.0
"""Clippy and rustfmt on a Rust target a rule builds through a transition.

`.bazelrc` runs Clippy and rustfmt as aspects on the targets a build
names, which `//...` reaches unless they are manual. A rule that builds
its Rust target through a transition, as `opt_test` does a board test
and the `vreteno_*` rules do a program for the core, names a target
that is manual, and so that target was never checked: a lint in a board
test reached `main` unseen (issue 1629), and so had a score of lints
and a misformatted file in the core's programs.

Such a rule puts `RUST_CHECK_ASPECTS` on the attribute it transitions
and returns `rust_check_groups` of it. The build's own
`--output_groups=+clippy_checks,+rustfmt_checks` then asks for the
checks of what the rule built, in the configuration it built it in.
"""

load("@rules_rust//rust:defs.bzl", "rust_clippy_aspect", "rustfmt_aspect")

RUST_CHECK_ASPECTS = [rust_clippy_aspect, rustfmt_aspect]

_GROUPS = ["clippy_checks", "rustfmt_checks"]

def rust_check_groups(targets):
    """The Clippy and rustfmt checks of `targets`, as an `OutputGroupInfo`.

    Args:
      targets: the targets the aspects ran on; a transition's attribute
        gives a list, and a target that is not Rust adds nothing.

    Returns:
      An `OutputGroupInfo` with the groups `clippy_checks` and
      `rustfmt_checks`.
    """
    groups = {}
    for group in _GROUPS:
        groups[group] = depset(transitive = [
            getattr(t[OutputGroupInfo], group)
            for t in targets
            if OutputGroupInfo in t and hasattr(t[OutputGroupInfo], group)
        ])
    return OutputGroupInfo(**groups)

def _rust_checked_impl(ctx):
    return [DefaultInfo(), rust_check_groups(ctx.attr.targets)]

rust_checked = rule(
    implementation = _rust_checked_impl,
    doc = "Clippy and rustfmt on manual Rust targets that nothing else " +
          "in `//...` builds, such as a test kept out of the suite for " +
          "its run time. Only the checks are built; nothing is run or " +
          "linked.",
    attrs = {
        "targets": attr.label_list(
            doc = "The Rust targets to check.",
            aspects = RUST_CHECK_ASPECTS,
            mandatory = True,
        ),
    },
)
