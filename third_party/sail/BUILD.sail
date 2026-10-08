# SPDX-License-Identifier: Apache-2.0
# The fetched Sail reference model: one static binary, which needs only
# the C library.
package(default_visibility = ["//visibility:public"])

filegroup(
    name = "sail",
    srcs = ["bin/sail_riscv_sim"],
)
