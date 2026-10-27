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
On one build machine (eph4, eight cores), from a warm cache, elaboration and Golden Gate take about 1.5 minutes, and the bitstream about 30 minutes, at a worst negative slack of +0.318 ns and 69% of the part's LUTs.
An opt-in shell built on rules_vivado's own rules, `firesim_nitefury_shell`, makes the same shell without a block design, so its cores and synthesis are cached apart.

The toolchain's README says how a consumer sets it up: the registry lines for `.bazelrc`, and what only the root module can state in `MODULE.bazel`, namely Scala 2.13.16, the C++ toolchain, Vivado and a lock file for the Maven set.

## What TxHDL takes of it today

Only Golden Gate, through one test (PR 1929):

* `MODULE.bazel` depends on the module `firesim_goldengate` 1.22.0, which holds Golden Gate prebuilt; the rule of the same name in rules_firesim runs it from there.
* `//firesim:goldengate_test` runs that prebuilt binary, `@firesim_goldengate//:goldengate`, directly and not through the rule, on FireSim's GCD example for the Nitefury II platform.
  The example is checked in under `firesim/testdata/`, GCD elaborated by FireSim 1.22.0's examples generator, about 22 KB of FIRRTL and annotations.
  The test checks that the Verilog and the driver header come out, with the simulator's top module, `FPGATop`, and GCD's own logic in the Verilog, and the register blocks of GCD's two bridges, peek-poke and the clock, in the header.

`//firesim/ax7a200b` is FireSim's GCD example on the AX7A200B with Ethernet as the host link instead of PCIe (#1931, milestone 1).
It takes FireSim's driver sources and example drivers from the module `firesim_target` 1.22.0.bcr.1.
On the board, `HostBridge` (`//lib/parts`, `remote::host`) takes register reads and writes off the Ethernet port in frames and performs them on the control port of Golden Gate's `FPGATop`, at 50 MHz.
On the host, `gcd_eth_driver` is FireSim's GCD test with a platform file, `simif_ax7a200b_eth.cc`, that sends those frames on a raw socket.
`gcd_eth_pnr` meets timing; the run on the board, from srv over the cable, is still to come.

`rules_firesim` itself is not a dependency yet.
It moves `rules_cc`, `dtc` and five more modules to newer releases, so it comes in with the first change that uses its rules, and that change checks what `bazel mod graph` moves.

## What comes next

Part (c) of #1927 is a FireSim platform for the AX7A200B, the board TxHDL runs on, reusing `//pcie`'s XDMA endpoint and `//ddr3`'s MIG controller.
As far as is known, the board has what FireSim needs: XDMA over its PCIe edge, Gen2 with two lanes in `//pcie` against the Nitefury II's four, so half the bandwidth, and DDR3 through `//ddr3`.
The published platform and the bitstreams built so far are for the Nitefury II, a different board with the same part, which TxHDL does not have.

A run on hardware needs more than a platform.
FireSim's host driver reaches the FPGA over PCIe, through XDMA's `/dev/xdma`, so the board must sit in a PCIe slot of a host that runs the driver.
Today it does not: it hangs off a host over JTAG and a serial line only, and TxHDL's own PCIe endpoint has not run in a host yet, as `//docs:pcie` says.
So the first platform for the board uses its Ethernet port instead (#1931), at a cost in speed: a register access takes tens of microseconds over a frame and its answer, against about one over PCIe.
Milestone 1 is FireSim's GCD example, whose bridges are registers only; milestone 2 is a Rocket core, which needs DRAM through `//ddr3` and FireSim's streams carried over frames.

## What it is not

FireSim here is a way to run a RISC-V design built by Chipyard far faster than a software simulator can.
It is not TxHDL's own flow: TxHDL's designs are lowered by `#[lower]` and built by `rules_vivado` directly, and nothing in `//lib` depends on FireSim.
