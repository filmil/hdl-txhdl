<!-- SPDX-License-Identifier: Apache-2.0 -->
# The path into DDR3: what limits it, and how to widen it

Status: design, October 4, 2026, for issue 1023, steps 2 and 3; section 8 records step 3.
Read against main at `c310333`; paths are at that commit.
Author: automated coding assistant, with human supervision.

Issue 1023 asks to measure the path every DDR3 user shares, then raise its throughput.
Step 1, the measurement, is `//ddr3:bw` in simulation and `ddr3bw_ram_bin` on the board (`docs/board-checks.md`, "The path into DDR3").
This note designs steps 2 and 3 from what step 1 found.
The board run gives the one number the design leaves as a symbol: `L`, the cycles AMD's controller takes to answer a read.
On the board it is about 23 (section 6).

## 1. What limits the path today

The path has three pieces.

* The link into `Ddr3Per` (`ddr3/src/lib.rs`): 32-bit data at 100 MHz, so 400 MB/s at most, one beat a cycle.
* The bridge `AxiWb` (`lib/parts/src/bus/wb.rs`): it takes one AXI burst at a time and puts each word on the Wishbone as its own request, waiting for the answer before the next.
* The wrapper `ddr3_wb32` (`ddr3/hdl/ddr3_wb32.v`) in front of the controller: one request in flight (`busy`); each word goes to the controller as a whole burst of eight words, the word repeated across it and masked, so a command moves 32 bytes to use 4.

`//ddr3:bw` measured the bridge's share: a word costs the memory's latency and four cycles more, and a burst adds about one cycle of its own.
The wrapper acknowledges a write the cycle after the controller takes it, so a write's latency is about two cycles; a read waits for the controller's data, `L` cycles.
So, today, at 100 MHz, reasoning from the wrapper for the writes and leaving the reads to the board:

| | Cycles a word | MB/s |
|---|---|---|
| Writes | about 6 | about 66 |
| Reads | `L + 4` | `400 / (L + 4)` |

On the board `L` is about 23, so the read path gives about 27 cycles a word, 14.8 MB/s.
The scanout reads 74 MB/s at 640 by 480 and 60 Hz, so it cannot be fed from DDR3 through this path at any plausible `L`, before anyone else asks for a byte.

The memory is not the limit.
It is 32 bits wide at 400 MHz on both edges, 3.2 GB/s at peak, and the controller moves 256 bits in each 100 MHz cycle of its user interface.
Today a word uses one such cycle's command and an eighth of its data.

## 2. What the fix has to do

Two things, in this order of gain.

1. **More than one word in flight.**
   A read's latency is paid once for many words, not once a word.
   This alone takes reads from `400 / (L + 4)` MB/s towards the link's 400 MB/s, since the controller accepts a command a cycle when it is ready.
2. **A controller burst for eight words, not one.**
   With the link's 32 bits, one word a cycle is all the link carries, so this does not raise the link-bound figure.
   What it does is use an eighth of the commands and of the memory's time per word: the memory then has room for refresh, for row changes and for every other user, and the path stops being the limit when the link widens later.

A third thing the issue lists, more than one AXI burst outstanding, comes free with the first: the link's identifiers already allow it, and the host side already issues ahead.

## 3. Three ways to do it

**A. Pipeline the Wishbone, keep everything else.**
`AxiWb` issues a word's request without waiting for the last one's answer, up to a depth, and matches answers to requests in order.
The wrapper takes a command every cycle the controller is ready, and keeps each read's lane in a FIFO until its data returns, since the controller answers reads in order.
Small and local: two files, both already this repository's.
It gives item 1 and not item 2: every word is still a whole controller burst.

**B. A bridge from the link straight to the controller's native interface.**
A new unit, in place of `AxiWb` and the wrapper's logic, cuts an AXI burst into commands of eight words aligned to 32 bytes.
A read command's 256 bits come back as up to eight beats; a write gathers up to eight beats with their strobes into one command's data and mask.
It gives both items.
It is the most new logic, written and checked here, and it is the controller's native protocol that it has to get right, including `app_rdy` and `app_wdf_rdy` falling for refresh.

**C. The controller's own AXI4 port.**
AMD's MIG 7 Series can be generated with an AXI4 slave interface in place of the native one (`<PortInterface>` in `ddr3/mig/ddr3_mig.prj`, now `NATIVE`).
It cuts bursts into commands, packs and masks, keeps reads outstanding by identifier, and is AMD's to maintain.
What it needs here is the mirror of `AxiPins`: a part that puts the link's peripheral end onto an external slave's pins, as `AxiPins` puts an external host's pins onto the link, and holds nothing.
The simulation model becomes an AXI memory, which `bus::axi::sim::Ram` already is, in place of the Wishbone one.
Two things to confirm by generating it, before choosing it: that the AXI port can be 32 bits wide with the controller's 256-bit native width behind it, and what it costs in LUTs and latency.

## 4. Recommendation

C, if the generated controller confirms a 32-bit AXI port; B otherwise; A only as a stopgap.

C is what this repository's rule asks for: of two interfaces, take the one somebody else maintains.
The controller's AXI port is maintained by AMD and widely used, AMD's own reference designs among them; the native protocol in B would be maintained by nobody but its author here.
C's own part, the mirror of `AxiPins`, holds no state and has a twin already checked.
A is cheap and gives most of the read gain, so it is the fallback if C or B stalls, but it spends eight times the memory's commands, which every later user pays for.

## 5. Staged, one pull request each

1. **Generate the controller with its AXI port**, as a second `vivado_ip` beside the native one, and record its width options, size and read latency in this note. Manual, as every Vivado target is.
2. **The mirror of `AxiPins`**, `AxiPerPins` say, in `lib/parts/src/bus/axi_pins.rs`, with a lowered example co-simulated as `ex_axi_pins` is.
3. **`Ddr3Per` on the AXI controller**: the new wrapper, the new model (`bus::axi::sim::Ram` with the controller's latency), and `//ddr3:bw` and `board_test`'s pinned numbers moved to what the new path gives.
4. **The board run again**, `ddr3bw_ram_bin` and the scanout's `starved` bit under load (`docs/board-checks.md`, "The video scanout under load"), and `Ddr3Per`'s datasheet given the new figures by the sweep rule.

If step 1 rules C out, steps 2 and 3 become B's unit and its tests, and step 4 is unchanged.

## 6. What the board measured

hil ran `ddr3bw_ram_bin` on the flagship on October 4, 2026, which has the same core, data memory and path into DDR3 as the boot bitstream, with the other hosts idle.
Sixteen loads took 841, 837, 834 and 826 cycles against the DDR3 and 397 against the data memory, about 27 cycles a word more; less the bridge's four, the controller adds `L` = 23 or so.
The stores took 250 against both memories, as the simulation predicted: the core cannot fill the path.
A second figure agrees: issue 1038's measurement has the Ethernet fetch engine reading frames from DDR3 at 12.7 MB/s, through the same path.

So reads today are about 15 MB/s, a fifth of what the scanout alone needs.
The design does not depend on `L`: any `L` above a few cycles leaves reads far below the scanout's need without item 1, and every option above provides it.

## 7. The controller's AXI port, generated

The user chose C on October 4, 2026, and the first of its steps is done: `//ddr3:ddr3_mig_axi` generates AMD's controller with its AXI4 port, from `ddr3/mig/ddr3_mig_axi.prj`, the native project file with `PortInterface` set to AXI.
It answers the question C was conditional on: the port can be 32 bits wide with the controller's wide native interface behind it.

What the generated top level, `ddr3_mig_axi.v`, offers:

| Parameter | Value |
|---|---|
| `C_S_AXI_DATA_WIDTH` | 32, the link's width |
| `C_S_AXI_ID_WIDTH` | 5, the link's identifier since issue 1041 |
| `C_S_AXI_ADDR_WIDTH` | 30, the memory's whole 1 GiB as bytes |
| `C_S_AXI_SUPPORTS_NARROW_BURST` | 0: every beat is the full 32 bits |
| `C_RD_WR_ARB_ALGORITHM` | `RD_PRI_REG`, the generator's default |
| `nCK_PER_CLK` and `DQ_WIDTH` | 4 and 32, so the native interface behind the port is 256 bits a cycle |

The port carries all five AXI4 channels with burst length, size, burst type, lock, cache, protection and quality of service, and the user clock and reset beside them.
It has no region signals, which the link carries, so the part that joins the link to these pins leaves the link's region unconnected.
Splitting a burst into the memory's commands and converting 32 bits to 256 are AMD's, inside the generated core; how many transactions it keeps outstanding is for step 3 to measure.

What generation alone cannot say is the port's size and its read latency.
Both come with step 3, which puts `Ddr3Per` on this port: its synthesis gives the size, and its simulation and then the board give the latency, which is what the path's throughput turns on.

## 8. `Ddr3Per` on the AXI port

Step 3 puts the design on the port of section 7.
`Ddr3Per` is now `AxiPerPins` in front of the controller, and the board's router port goes straight to it, with no tracker in front, since the controller keeps its own transactions.
The wrapper is `ddr3/hdl/ddr3_axi32.v`.
It drops the link's top two address bits, which the design has already decoded, and gives the controller its AXI reset from the user clock's.
`AxiWb` and `ddr3_wb32` are no longer on the path.

**The controller's latency, in simulation.**
`//ddr3/sim:wrapper_test` runs the wrapper and the controller against the Micron models, in cycles of the 100 MHz user clock:

| What | Cycles |
|---|---|
| A read's first beat, after its address phase is taken | 25 |
| The same, for a burst of sixteen | 27 |
| The same, for the first read right after a write | 63 |
| A write's response, after its last beat is taken | 3 |

It also writes and reads a byte at an address that is not a word's, under that byte's strobe, as the core's byte accesses are; the controller was generated without narrow bursts, and a full-width beat at a byte's address is what it is given and what it handles.

The testbench issues one transaction at a time, so it does not measure how many the controller keeps outstanding.
The model in `ddr3::Ddr3` answers with the common case: a read's first beat 24 steps after it sees the address, which is a cycle after the address is taken, and a write's response 2 steps after its last beat.

**The path, in simulation.**
`//ddr3:bw` now measures `AxiPerPins` in front of a memory on its pins at a given latency, and `Ddr3Per` with its model:

| | Cycles a word | MB/s at 100 MHz |
|---|---|---|
| Reads, four bursts of sixteen in flight, at any latency up to 32 | 1.01 to 1.04 | 386 to 398 |
| Reads, one burst at a time, at latency 24 | 2.81 | 142 |
| Writes, at any latency up to 32 | 1.06 to 1.09 | 365 to 376 |
| `Ddr3Per` with its model, reads | 1.03 | 390 |
| `Ddr3Per` with its model, writes | 1.06 | 376 |

A read pays the latency once a burst rather than once a word, and with bursts in flight not even that.
A write costs a beat a cycle, and one step a burst once the testbench client's window is full and it waits on its oldest burst before issuing the next.
Until issue 1121 the client also waited three steps a burst for its identifier before sending the first beat, which made writes 1.25 cycles a word here.
That wait was the simulation client's, not a hardware host's.
Against section 1's 27 cycles a word for reads on the board, that is the gain the issue asked for, if the board agrees.

The core's own timing of the path, `the_ddr3_path_is_timed_by_the_core`, now finds a DDR3 load costing the model's read latency less two cycles more than a data memory load, since the pins part and the port take two cycles fewer than the data memory's tracker and block RAM.
The core's stores take the same cycles into either memory, since the core issues them no faster than it fetches the routine from the data memory; at a write latency of 4 rather than the controller's 2 the DDR3's came out 14 cycles faster in sixteen, so the stores say how the core issues them and not how fast the path is.

**Size and timing.**
`//flagship:flagship_pnr` on this branch, against the same target at main 36201f8, whose design is main's at the time of writing, measured the same way:

| | Main 36201f8 | With the AXI port |
|---|---|---|
| Worst setup slack, `clk_pll_i` | +0.106 ns | +0.055 ns |
| Worst hold slack | +0.032 ns | +0.031 ns |
| LUTs | 21,315 | 23,347 |
| Flip-flops | 19,786 | 21,959 |
| Block RAM tiles | 18.5 | 18.5 |
| DSPs | 4 | 4 |

Every constraint is met in both.
The worst setup path is the core's own in both, a register to the boot memory's address (`mstatus` here), and not the controller's; the core's slack swings by about 0.25 ns with placement alone (issue 1130), so the 0.05 ns it moved is within that.
It is also within 0.06 ns of failing, which is the core's margin and what issue 1130 is about.
The summary names only the worst path of each clock, so it does not say how close the controller's own paths come; its crossings to the other clocks have 5.9 ns or more.
The design is about 2,000 LUTs and 2,200 flip-flops larger, which is the AXI port less what `AxiWb`, `ddr3_wb32` and the tracker in front of them took, and no block RAM larger.


What the board says is step 4.
