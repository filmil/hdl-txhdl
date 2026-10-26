# FireSim in TxHDL

FireSim simulates a RISC-V system on an FPGA: the design runs as hardware, at speed, while a host steps it and reads it through drivers.
Its flow used to need sbt, conda and a Vivado project on the host.
It now builds with Bazel, from source, as a toolchain of its own that TxHDL uses as a consumer (#1927).
This page says what that toolchain is, what TxHDL takes of it today, and what comes next.

## The toolchain

The toolchain is published apart from TxHDL:

* [filmil/bazel_rules_firesim](https://github.com/filmil/bazel_rules_firesim) v0.1.0, the rules;
* [filmil/bazel_firesim_goldengate](https://github.com/filmil/bazel_firesim_goldengate) 1.22.0, FireSim's Golden Gate compiler, built in a repository of its own;
* 22 modules in the [filmil registry](https://github.com/filmil/bazel-registry), most of them overlays on the upstream commits Chipyard 1.15.0 pins.

It targets FireSim 1.22.0, and has four rules:

| Rule | What it makes |
|---|---|
| `firesim_elaborate` | The target design, as legacy SFC FIRRTL, from Chipyard's generator |
| `firesim_goldengate` | Golden Gate's output: `FireSim-generated.sv`, the driver header and the constraints |
| `firesim_driver` | The host driver for one design and platform |
| `firesim_bitstream` | The bitstream, by the board project's own build script under rules_vivado |

Golden Gate is a separate Scala world from the target, Chisel 3.6.1 against the target's Chisel 6.7.0.
rules_scala takes its Scala version from the root module alone, so Golden Gate is built in its own repository and used as a prebuilt tool.

The one platform is FireSim's RHS Research Nitefury II (`xc7a200tfbg484-2`), on Vivado 2025.2.
FireSim's own flow for it is pinned to Vivado 2022.1; the change 2025.2 needs, XDMA 4.1 to 4.2, is a patch in `firesim_platform_nitefury`.
FireSim's `nitefury_firesim_rocket_singlecore_no_nic` recipe goes from Chisel to a bitstream that meets timing.
From a warm cache on eph4, elaboration and Golden Gate take about 1.5 minutes, and the bitstream about 30, at a worst negative slack of +0.318 ns and 69% of the part's LUTs.
An opt-in shell built on rules_vivado's own rules, `firesim_nitefury_shell`, makes the same shell without a block design, so its cores and synthesis are cached apart.

The toolchain's README says how a consumer sets it up: the registry lines for `.bazelrc`, and what only the root module can state in `MODULE.bazel`, namely Scala 2.13.16, the C++ toolchain, Vivado and a lock file for the Maven set.

## What TxHDL takes of it today

Only Golden Gate, through one test (PR 1929):

* `MODULE.bazel` depends on `firesim_goldengate` 1.22.0, the module rules_firesim runs Golden Gate from.
* `//firesim:goldengate_test` runs Golden Gate on FireSim's GCD example for the Nitefury II platform.
  The example is checked in under `firesim/testdata/`, GCD elaborated by FireSim 1.22.0's examples generator, about 22 KB of FIRRTL and annotations.
  The test checks that the Verilog and the driver header come out, with the simulator's top module, `FPGATop`, and GCD's own logic in the Verilog, and the register blocks of GCD's two bridges, peek-poke and the clock, in the header.

`rules_firesim` itself is not a dependency yet.
It moves `rules_cc`, `dtc` and five more modules to newer releases, so it comes in with the first change that uses its rules, and that change checks what `bazel mod graph` moves.

## What comes next

Part (c) of #1927 is a FireSim platform for the AX7A200B, the board TxHDL runs on, reusing `//pcie`'s XDMA endpoint and `//ddr3`'s MIG controller.
A run on hardware needs more than a bitstream: FireSim's host drives the design over PCIe, so the board must sit in a PCIe slot of a host that runs the driver.
TxHDL's PCIe endpoint has not run in one yet; `//docs:pcie` says its validation in a host remains to be done.
So (c) starts with a build that meets timing, and a run follows once the board is in a host.

## What it is not

FireSim here is a way to run a RISC-V design built by Chipyard far faster than a software simulator can.
It is not TxHDL's own flow: TxHDL's designs are lowered by `#[lower]` and built by `rules_vivado` directly, and nothing in `//lib` depends on FireSim.
