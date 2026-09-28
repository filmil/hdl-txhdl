# SPDX-License-Identifier: Apache-2.0
"""The C and C++ toolchain, and the sysroot it compiles against.

LLVM from `toolchains_llvm`, over a Debian bullseye sysroot unpacked
from packages pinned by checksum (issue 782). Both repositories are
made here, in one extension of this module's own, for two reasons.

`toolchains_llvm`'s own extension refuses to run in a module that is not
the root, and this module is not the root for a workspace that depends
on it, as the blinky of `//docs:zero` does. Its `llvm_toolchain` macro
has no such limit.

And the toolchain has to name the sysroot by a label that resolves from
inside `toolchains_llvm`, where this module's repositories are not
visible. A label made here and turned into its canonical form does
resolve there, whether this module is the root or a dependency, which a
label written as a string in `MODULE.bazel` cannot do.
"""

load("@toolchains_llvm//toolchain:rules.bzl", "llvm_toolchain")
load("//third_party/debs:repo.bzl", "deb_tree")

# The LLVM release. toolchains_llvm fetches it by the checksum it keeps
# for each release.
LLVM_VERSION = "20.1.8"

def _cc_toolchain_impl(module_ctx):
    deb_tree(
        name = "sysroot_bullseye",
        locks = [Label("//third_party/sysroot:bullseye.lock.tsv")],
        sysroot = True,
    )
    llvm_toolchain(
        name = "llvm_toolchain",
        llvm_versions = {"": LLVM_VERSION},
        sysroot = {
            "linux-x86_64": str(Label("@sysroot_bullseye//tree:sysroot")),
        },
    )
    return module_ctx.extension_metadata(reproducible = True)

cc_toolchain = module_extension(implementation = _cc_toolchain_impl)
