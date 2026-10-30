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
`gcd_eth_pnr` meets timing, with 0.534 ns of setup slack.

It passed on the board on 2026-10-27 at `main` 83a5613b, from srv over the cable; the output is on #1931.
The driver read the simulator's fingerprint, stepped GCD on 64 and 48, read back 16, and printed `[GCD]PASS Test`, 12 cycles in all.
To repeat it, holding the BOARD TOKEN:

1. Program the board with `gcd_eth_prog`, as the comment beside it in `firesim/ax7a200b/BUILD.bazel` says.
2. Build the driver with `bazel build //firesim/ax7a200b:gcd_eth_driver`, copy `bazel-bin/firesim/ax7a200b/gcd_eth_driver` to srv, which works since it needs no library beyond libc, and let it open a raw socket: `sudo setcap cap_net_raw+ep gcd_eth_driver`.
3. Run it on the port cabled to the board, with nothing else sending there: `timeout 120 ./gcd_eth_driver +iface=fpga-a200t-eth0`.

It passes when it exits 0 and prints `PASS Test`.
`no answer to a read at 0x0` means no frame came back: the bitstream isn't loaded, or the interface is wrong.

Milestone 2's streams have their path over frames (#1984).
FireSim's driver moves a CPU-managed stream with `cpu_managed_axi4_read` and `cpu_managed_axi4_write`, on `FPGATop`'s 128-bit AXI4 port.
`StreamBridge` (`//lib/parts`, `remote::stream`) carries those one 16-byte beat per frame, under EtherType 0x88b6, so `EthShare` can split them from `HostBridge`'s frames by type.
`simif_ax7a200b_eth.cc` implements both calls with those frames, and `eth_frame.h` lays out both kinds of frame.
A stream read or write can't be repeated or skipped, so the driver's first stream ask is a no-op, which sets the tag the board keeps.
`ex_stream` in `//docs:examples` shows the frames on a queue.

Milestone 2, FireSim's Rocket design, passed on the board on 2026-10-27 (#1988).
The design is `rocket_nitefury-v0.2.1.tar.gz`, Golden Gate's outputs for `FireSimRocket1GiBDRAMConfig`, prebuilt by bazel_rules_firesim's CI and fetched by sha256 as `@firesim_rocket_nitefury`, so TxHDL needs no Scala.
`rocket_eth_top.v` joins all three of `FPGATop`'s ports:

* the control port and the stream port to `rocket_link`, which splits the frames by EtherType between `HostLink` with `LiteMaster` and `StreamBridge`;
* `mem_0`, the target's DRAM, through an AXI clock converter to `rocket_mig`, `//ddr3`'s MIG with its AXI port at `FPGATop`'s 64-bit data and 16-bit IDs.

`rocket_eth_pnr` meets timing with 0.096 ns of setup slack, in the L2 cache's logic on the simulator's clock, and uses 60% of the LUTs.
The driver, `rocket_eth_driver`, loaded `hello.riscv` into the DDR3 over TSI; the target printed `Hello from Rocket on the AX7A200B, over Ethernet.` on its UART and ended with `*** PASSED ***` after 68,162,012 target cycles.
The bitstream merged with PR 1991, whose host link waits for the simulator's reset, passed the same run again the same evening, after 48,114,362 target cycles.
The count differs from run to run because the target waits in its boot ROM while the driver loads the program over Ethernet, so it measures the host's pace more than the design's.
To repeat it, holding the BOARD TOKEN:

1. Program the board with `rocket_eth_prog`, as the comment beside it says.
2. Build `//firesim/ax7a200b:rocket_eth_driver` and `:hello_riscv`, copy both to srv, and let the driver open a raw socket: `sudo setcap cap_net_raw+ep rocket_eth_driver`.
   The driver is one file: it links Spike statically, through `riscv_isa_sim` `0.0.0-chipyard.1.15.0.bcr.1` (#2001).
   The board runs of #1988 predate that and needed Spike's shared libraries beside a driver linked with an absolute `-rpath`, since a binary given `cap_net_raw` ignores `LD_LIBRARY_PATH`.
3. Run `./rocket_eth_driver +permissive +iface=fpga-a200t-eth0 +permissive-off hello.riscv`.
   The driver's own plusargs go inside the `+permissive` block; outside it, fesvr takes the first one for the program.
   `+device` defaults to 5, the device number `rocket_netlist.rs` builds in.

FireSim expects a freshly programmed FPGA for each run, so the driver resets the simulator before each one (#2007).
A run that ends leaves its step unfinished: the driver writes `STEP` once, and after it exits the TSI bridge stops the target until a host serves it.
`STEP` is a queue that takes a write only once the step before has finished, so without a reset the fourth run on one programming waited for ever on its `STEP` write, and nothing on the control path answered after it.
The driver's first frame is a reset frame, kind 0x04 on EtherType 0x88b5 with the device number at byte 15, and it gets no answer.
`ResetTap`, in front of `EthShare` in `rocket_link`, passes every frame on and raises `reset` at the end of one; it reads frames as they go, so it sees a reset while `LiteMaster` is stuck.
`rocket_eth_top.v` then starts the MIG's power-on reset again, and the MIG's calibration holds the clock converter, `FPGATop` and the link in reset, as at power-on.
The driver gives its first ask five seconds for an answer while the memory calibrates.

`+eth_log=<file>`, inside the `+permissive` block, logs every register access and stream beat with its time, plus each ask sent again and each ask that got no answer.
On a board that stops answering, the log's last lines name the access it stopped at.

Linux boots on the Rocket design (#2015).
`//firesim/ax7a200b/linux:rocket_linux` is one ELF: OpenSBI 1.6, with Linux 6.18.55 inside it and a BusyBox initramfs inside that.
`//linux`'s scripts build it, as they build Vreteno's Linux, with the same hermetic LLVM, for RV64.
The kernel is `tinyconfig` with `rocket.config` over it, for what the device tree in the design's boot ROM describes: one RV64 hart with Sv39, the CLINT, the PLIC, a SiFive serial port at 0x10020000 and 1 GiB of DRAM.
`/init` shows the machine and powers off, and OpenSBI passes the power-off to the driver through `tohost`.
To repeat it, with the board programmed and the driver set up as above:

```sh
bazel build //firesim/ax7a200b/linux:rocket_linux
# copy bazel-bin/firesim/ax7a200b/linux/rocket_linux.elf beside the driver, then:
./rocket_eth_driver +permissive +iface=fpga-a200t-eth0 +permissive-off rocket_linux.elf
```

It passes when the console shows `txhdl: userspace is up on Rocket`, the driver prints `*** PASSED ***` and exits 0.
On 2026-10-30 the whole run took 4 min 36 s and 288,686,162 target cycles: 4 min 22 s to load the ELF's 3.45 MB, and 13 s from OpenSBI's banner to the power-off.
The load is slow because FireSim writes the memory four bytes per register write, and each write is one round trip on the cable, about 0.3 ms.

Two things had to match FireSim:

* The console runs at 3686400 baud, the speed the serial port's divisor resets to on the 1 GHz bus, which FireSim's UART bridge decodes.
  At Linux's default of 115200, every byte after the console's handover arrived as a NUL.
* OpenSBI links itself position independent, and FireSim's front end loads such an ELF 0x80000000 higher.
  The DRAM's alias still took the load, but the front end polled `tohost` at 0x1_00042a90.
  That run took 18 min and 36.0 billion target cycles, and ended about two minutes after the kernel's power-off.
  `build.sh payload` marks the ELF an executable, since its segments are at their run addresses already.

`rules_firesim` itself is not a dependency yet; it comes in with the first change that uses its rules.
`firesim_target` has already moved `rules_cc`, `dtc` and five more modules to the newer releases they share.

## What comes next

Part (c) of #1927 is a FireSim platform for the AX7A200B, the board TxHDL runs on, reusing `//pcie`'s XDMA endpoint and `//ddr3`'s MIG controller.
As far as is known, the board has what FireSim needs: XDMA over its PCIe edge, Gen2 with two lanes in `//pcie` against the Nitefury II's four, so half the bandwidth, and DDR3 through `//ddr3`.
The published platform and the bitstreams built so far are for the Nitefury II, a different board with the same part, which TxHDL does not have.

A run on hardware needs more than a platform.
FireSim's host driver reaches the FPGA over PCIe, through XDMA's `/dev/xdma`, so the board must sit in a PCIe slot of a host that runs the driver.
Today it does not: it hangs off a host over JTAG and a serial line only, and TxHDL's own PCIe endpoint has not run in a host yet, as `//docs:pcie` says.
So the first platform for the board uses its Ethernet port instead (#1931), at a cost in speed: a register access takes tens of microseconds over a frame and its answer, against about one over PCIe.
Milestone 1 was FireSim's GCD example, whose bridges are registers only; milestone 2, a Rocket core with its DRAM on `//ddr3` and FireSim's streams over frames, passed as well, and that core boots Linux.

## What it is not

FireSim here is a way to run a RISC-V design built by Chipyard far faster than a software simulator can.
It is not TxHDL's own flow: TxHDL's designs are lowered by `#[lower]` and built by `rules_vivado` directly, and nothing in `//lib` depends on FireSim.
