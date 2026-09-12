# SPDX-License-Identifier: Apache-2.0
"""A Verilog module as a unit: `verilog_unit()`."""

load("@rules_cc//cc:cc_library.bzl", "cc_library")
load("@rules_rust//rust:defs.bzl", "rust_library")
load("@rules_verilator//verilator:defs.bzl", "verilator_cc_library")
load("@rules_verilog//verilog:defs.bzl", "verilog_library")

def verilog_unit(name, src, top, clock = "clk"):
    """A Verilog module as a unit, in a crate of its own.

    `src` is the Verilog file, `top` the module in it, `clock` its
    clock port. Verilator compiles the module into a C++ model;
    `//tools/vshim` writes a C shim over the model and the Rust that
    makes a `Unit` of it, named after the module in CamelCase, with an
    `In` or `Out` per port and an `Rx` or `Tx` per `x_data`, `x_valid`,
    `x_ready` trio; and the crate `name` holds that unit, for an
    example or a design to depend on."""
    verilog_library(
        name = name + "_vl",
        srcs = [src],
        top_module = top,
    )
    verilator_cc_library(
        name = name + "_verilated",
        module = ":" + name + "_vl",
        vopts = ["-Wno-fatal"],
    )
    native.genrule(
        name = name + "_gen",
        srcs = [src],
        outs = [name + "_shim.cc", name + ".rs"],
        cmd = "$(location //tools/vshim) $(location " + src + ") " + top +
              " " + clock + " " + name + " $(location " + name + "_shim.cc)" +
              " $(location " + name + ".rs)",
        tools = ["//tools/vshim"],
    )
    cc_library(
        name = name + "_shim",
        srcs = [name + "_shim.cc"],
        deps = [":" + name + "_verilated"],
    )
    rust_library(
        name = name,
        srcs = [name + ".rs"],
        crate_name = name,
        crate_root = name + ".rs",
        edition = "2021",
        deps = ["//lib:txhdl", ":" + name + "_shim"],
    )
