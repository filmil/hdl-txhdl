<!-- SPDX-License-Identifier: Apache-2.0 -->
# On-chip debugging for the SoC

Status: analysis, September 18, 2026; step 1 of section 4 built on
September 20, 2026, under issue 239.
Author: automated coding assistant, with human supervision.

This note answers issue 136: what the options are for debugging the SoC
on the board, what each one costs, and which to do first.
Every statement about the tree here was read out of it, and says where.

## 1. What there is today

On the board, a running design shows four things and no more.

* Four LEDs, driven by `cpu/vreteno/board/vreteno_board.v:157-160`:
  the core's halt, whether the serial line has ever started a byte,
  the DDR3 controller's calibration, and a heartbeat off bit 25 of a
  counter.
* A serial line at 115200 baud on `uart_tx` L15 and `uart_rx` L14
  (`cpu/vreteno/board/ax7a200.xdc:17-18`), watched by a Go program
  uploaded over ssh, `//cpu/vreteno/board/remote:serial`.
* Whatever the program prints over that line.
* JTAG, used to configure the part and for nothing else; the remote
  `hw_server` is started with `-l jtag -l jtag2`
  (`cpu/vreteno/board/remote/hw_server_remote.sh:24`).

Inside the design there is more state than that, and it does not leave.
The core exposes `halt`, `instr`, the retiring instruction word, and
`wb`, a `Writeback { done, rd, val }` (`cpu/vreteno/src/core.rs:610-627`
and `:1104-1110`).
The board unit makes the `instr` and `retire` signals and connects them
to nothing: `cpu/vreteno/src/board.rs:134-135` creates them and no port
carries them out.
So the retiring instruction and the register it wrote are already
computed on the board, and are thrown away a wire later.

Three more facts decide what is cheap and what is not.

* The core has no halt, resume or step input.
  Its only inputs are reset and two interrupt lines
  (`cpu/vreteno/src/core.rs:610-627`).
* `ebreak` halts the machine for good.
  It latches `stopped` and raises `halt`, and nothing clears it but
  reset (`cpu/vreteno/src/core.rs:836`, `:870-873`, `:915`, `:1063`).
  That is issue 139.
* There are eight CSRs, `mstatus` through `mip`
  (`cpu/vreteno/src/core.rs:448-479`).
  None of the debug specification's CSRs exist: no `dcsr`, no `dpc`,
  no trigger registers.
* Nothing loads a program at run time.
  The instruction and data memories are initialised in the netlist
  (`cpu/vreteno/src/bin/board_netlist.rs:10-11`), so changing the
  program means a synthesis and a bitstream.

Against that, the simulator sees everything.
A run records every register, both ends of every wire and all six
signals of every channel (`lib/src/comp.rs:1274-1520`), and
`//cpu/vreteno:lockstep_test` compares the architectural PC, all 31
registers, the halt bit and all eight CSRs against a reference model
every cycle (`cpu/vreteno/tests/lockstep.rs:258-303`).
The netlists are then checked against those traces under nvc and
Verilator.
Most bugs are caught there, which is why the board has needed so little.
What the board has caught is the other kind: a bitstream that runs and
says nothing (issue 195), where the question is not what the program
did but whether a wire reaches a pin.

## 2. The three things "debugging" means here

They are separate needs, and an option that serves one need may serve
neither of the others.

1. **Look at a design that is stuck.**
   The part is configured, the LEDs say something unhelpful, and the
   question is which wire is wrong.
   This is a signal-level question about hardware.
2. **Look at a program.**
   The core runs, and the question is what it executed, what it wrote,
   and where it went wrong.
   This is an architectural question, and it wants the retire stream,
   memory and registers.
3. **Change what runs without a resynthesis.**
   Loading a program, restarting it, patching a word.
   A synthesis and a place and route of this design is measured in tens
   of minutes; a load over a wire is measured in seconds.

## 3. The options

### 3.1 An ILA, which the build already has rules for

`rules_vivado` 3.10.2, the version in `MODULE.bazel:155`, ships
`vivado_ila` and `vivado_read_ila`.
`vivado_ila` takes the probe widths, a capture depth from 1024 to
131072 samples, storage qualification and trigger settings, and builds
the core through `vivado_ip`, which this tree already uses for the PCIe
endpoint (`pcie/BUILD.bazel:7`).
`vivado_read_ila` reads a capture back using the bitstream's `.ltx`
probe file.
Nothing in this repository uses either.

What it gives: need 1, exactly.
Any net in the design, at the design's own clock, with a trigger, read
back over the JTAG cable that is already connected.

What it costs: block RAM for the capture buffer, an implementation run
per set of probes, and a bitstream that is not the shipping one.
The probes are chosen before synthesis, so a question nobody thought of
costs another run.
The part has room: the board design uses 8100 LUT, 6959 FF and four
block RAM tiles (`docs/vreteno.tex:1189-1200`), on a part where the
DDR3 controller and the core together leave most of the fabric unused.

This is the smallest step, because the tool and the rules exist and the
design does not change.

### 3.2 The retire stream, which the design already computes

The core's `instr` and `wb` ports say, every cycle, which instruction
retired and what it wrote.
On the board they go nowhere.

A lowered unit could take them, pack them into words, and put them
somewhere a host can read: a ring buffer in block RAM read over the
existing AXI-Lite map, or a stream out of the UART, or into DDR3, which
the board already has.
That is a part in `//lib/parts`, an example that checks it against its
trace, and a peripheral in the board's router, all of which this
repository knows how to do.

What it gives: need 2, for a core that is still running.
An execution trace of the last N instructions, which is what a
post-mortem wants.

What it costs: the buffer's memory, a few hundred LUTs, and a decision
about bandwidth.
At 100 MHz a retire stream is up to 800 MB/s; a UART at 115200 baud
carries 11 kB/s.
So the buffer must be a window, triggered, or compressed to a PC
difference.

This is the option this repository is best shaped for, since it is one
more part on the bus.

### 3.3 JTAG to the SoC bus, without a debug module

A `BSCANE2` primitive gives user logic the JTAG chain that is already
wired to the board's USB cable, with no extra pins.
Nothing in the tree instantiates it today: no `BSCANE2`, `STARTUPE2`,
`DNA_PORT` or `ICAP` anywhere, only `IBUFDS`, `PLLE2_BASE` and `BUFG`
(`cpu/vreteno/board/vreteno_board.v:67-97`).
A bridge from that chain to the board's AXI router would let a host
read and write every address the map already names: the data memory,
the timer, the UART, the PLIC, the DDR3 window
(`cpu/vreteno/src/board.rs:46-60`).
Vivado's own JTAG-to-AXI Master is the same idea bought rather than
built, and `vivado_ip` can generate it.

What it gives: needs 2 and 3, for memory and peripherals.
A host could load a program into the data memory, read it back and
check it, and read any peripheral register while the core runs.
It does not stop the core and does not show its registers.

What it costs: a bridge to write, or an IP to generate and wire, and a
host-side script.
The instruction memory is inside the core and is not on the bus
(`cpu/vreteno/src/core.rs:580`), so loading a program this way wanted
issue 134, fetch from the bus.
That has since landed: the core fetches from the bus above its boot
memory, so a program written into memory runs, and what is left is the
boot memory itself, which is issue 268.

### 3.4 The RISC-V debug module over JTAG, issue 154

The standard answer, and the largest.
It wants, in the core: a halt request input and a resume, a single
step, the debug CSRs `dcsr` and `dpc`, entry to debug mode rather than
a permanent stop on `ebreak`, and abstract commands to read and write
registers and memory while halted.
It wants, beside the core: a debug module with a DMI, and a debug
transport over `BSCANE2`.
Then `openocd` and `gdb` attach, and every ordinary debugger workflow
follows.

What it gives: all three needs, properly.

What it costs: the core changes above, which touch the halt path and
the CSR file, and a specification to follow rather than invent.
It is the right end state and the wrong first step.

### 3.5 A software monitor in the program

A stub in the program that reads commands from the UART and answers
with memory and register contents.
It needs no hardware at all.

What it gives: need 2, and some of need 3, while the program is alive
and the interrupt path works.

What it costs: program space, of which there is 4 KiB
(`tools/elf2vreteno/main.rs:19-24`), and it sees nothing when the core
is stuck, which is the case that sends people looking for a debugger.

### 3.6 What the simulator already gives

Before any of this: a failing program can be run under the lockstep
test, and a failing unit under its co-simulation, where every register
of every cycle is visible and the reference model says which cycle went
wrong.
The board is worth reaching for when the question is about the board:
a pin, a clock, a PHY, a memory controller's calibration.

## 4. The order to do them in

1. **An ILA bitstream for the board design**, as a manual target
   beside the shipping one, which is issue 239.
   It is the smallest change that answers "which wire is wrong", the
   rules exist, and it costs no design change.
   Built: `//cpu/vreteno:vreteno_board_ila_pnr`, with thirteen probes
   listed and explained at the head of
   `cpu/vreteno/board/vreteno_board_ila.v`, and
   `//cpu/vreteno:vreteno_board_ila_read` to read a capture back.
   The retire stream is not among the probes, because `instr` and
   `wb` end inside the lowered module and are not its ports; that is
   step 2's work.
   A capture was read on September 22, 2026, in an unattended board
   window, and is in `docs/ila_capture.vcd` and `docs/vreteno.tex`; the
   rule's generated read script needed two fixes upstream first, which
   are bazel_rules_vivado#134.
2. **The retire stream as a part**, issue 240, since the core already
   computes it and the board already throws it away.
   It is this repository's own shape of answer, and it makes the
   execution history of a run readable after the fact.
3. **A JTAG path to the bus**, issue 241, which turns a synthesis into
   a download and feeds the bootloader of issue 135 and fetch from the
   bus of issue 134.
   Built: Vivado's JTAG-to-AXI master on a second host port of the
   board, behind an arbiter, `//cpu/vreteno:vreteno_board_jtag_pnr`,
   with `//cpu/vreteno:vreteno_board_jtag_probe` reading and writing
   the memories and the timer from the hardware manager; proved on
   the board on September 22, 2026.
4. **The debug module of issue 154**, once the core has halt, resume
   and step, which is also what issue 139 wants `ebreak` to become.

Steps 1 and 2 are independent of each other and of the core.
Steps 3 and 4 are the ones that change the core, and 4 subsumes much of
3.

## 5. What the core needs before step 4

Collected here so that the work is visible when it is taken:

* a halt request input and a resume, and the halt made clearable,
  which today it is not (`cpu/vreteno/src/core.rs:915`);
* single step, one instruction per resume;
* `dcsr` and `dpc`, and `ebreak` entering debug mode rather than
  stopping for good, which is issue 139's complaint from the other
  side;
* abstract access to the register file and to memory while halted,
  which the register file's port count decides.
