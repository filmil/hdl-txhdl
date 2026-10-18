# SPDX-License-Identifier: Apache-2.0
"""A bitstream given as a file, for a programming rule to take (#1784).

`vivado_program_device` programs the bitstream of the target it is
given, and every target that provides one, a place and route, builds
it. This rule provides a file as that bitstream instead, so that a
program script can be generated without a place and route, and a
wrapper can put another session's checked bitstream in its place.
"""

load("@rules_vivado//internal:providers.bzl", "VivadoBitstreamProvider")

def _vivado_given_bitstream(ctx):
    bit = ctx.file.src
    return [
        DefaultInfo(files = depset([bit])),
        VivadoBitstreamProvider(bitstream = bit, probes = None),
    ]

vivado_given_bitstream = rule(
    implementation = _vivado_given_bitstream,
    attrs = {
        "src": attr.label(
            allow_single_file = [".bit"],
            mandatory = True,
            doc = "The file that stands for the bitstream.",
        ),
    },
    doc = "Provides a file as a bitstream, with no place and route.",
)
