# SPDX-License-Identifier: Apache-2.0
"""The tools `build.sh` and `userspace.sh` run with, for every Linux built here.

Vreteno's Linux in //linux and the Rocket core's in
//firesim/ax7a200b/linux (issue 2015) both use them.
"""

LINUX_TOOLS = [
    "//linux:build.sh",
    "//linux:userspace.sh",
    "@kbuild_tree//:all",
    "@llvm_toolchain_llvm//:bin",
    "@llvm_toolchain_llvm//:lib",
    "@llvm_toolchain_llvm//:include",
    "@sysroot_bullseye//tree:sysroot",
    "@python_3_12_host//:python",
]

# The trees' files; build.sh finds where each begins from them.
LINUX_ENV = " ".join([
    "KBUILD_FILES='$(execpaths @kbuild_tree//:all)'",
    "LLVM_FILES='$(execpaths @llvm_toolchain_llvm//:bin)'",
    "SYSROOT=$(execpath @sysroot_bullseye//tree:sysroot)",
    "PYTHON3=$(execpath @python_3_12_host//:python)",
])
