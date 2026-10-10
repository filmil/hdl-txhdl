<!-- SPDX-License-Identifier: Apache-2.0 -->
# The board checks that are owed

Status: written September 26, 2026, while the board server was unreachable.
Author: automated coding assistant, with human supervision.

Twelve issues have waited on the AX7A200B, and each of them says what it wants from the board in its own comments.
This file puts those wants in one place, as commands to run in order, with what each should print and what to keep.
Three are done, #143, #153 and #864, and two in part, #188 and #458, as their sections record; the rest wait on a board session, and section 9 says what the two that cannot run yet still lack.

Nothing here writes the flash or touches the board until the user says the board server is back.

## 1. Before the session: build everything

First of all, each bitstream must meet timing.
Every place and route target checks it after routing and fails the build when the worst setup or hold slack is negative (#759), so a build below that ends in an error has found a timing miss, and nothing it would have built is programmed.
The console shows the error, `timing is not met`, with both slacks, under Vivado's errors and the end of its log (#761).
A passing build leaves the timing summary and the worst paths in `bazel-bin/<package>/<target>.timing_summary.pnr.rpt`.

A small positive hold slack is normal, since the router pads hold paths to just above zero; a negative one is the router having failed, and is a bug to file, not a board to try.

Every target below is manual, so `bazel build //...` does not build it.
Build them on the day before, one Vivado build at a time, since two at once exhaust this host's memory.

```sh
bazel build //cpu/vreteno:vreteno_board_jtag_pnr   # the debug module, #154
bazel build //cpu/vreteno:vreteno_board_boot_pnr   # the loader, #550 and #458
bazel build //cpu/vreteno:vreteno_board_pnr        # the DDR3 test, #188
bazel build //flagship:flagship_pnr                # Ethernet and the loader, #143
bazel build //cpu/vreteno/rust:hello_ram_bin //cpu/vreteno/rust:hello_fastboot \
    //cpu/vreteno/rust:trng_ram_bin //cpu/vreteno/rust:steps_ram_bin \
    //cpu/vreteno/rust:ddr3bw_ram_bin //cpu/vreteno/rust:ddr3ram_ram_bin \
    //cpu/vreteno/rust:cpi_ram_bin
bazel build //zephyr:fastboot @multitool//tools/fastboot
bazel build //third_party/gdb //cpu/vreteno/rust:gdbprobe_elf   # gdb, #872
bazel build //tools/trngstat
bazel build //cpu/vreteno/rust:phyregs_ram_bin       # the PHY's registers, #864
bazel build //cpu/vreteno/rust:phydelay_ram_bin      # the PHY's delay bits, #869
bazel build //cpu/vreteno/rust:sdprobe_ram_bin       # the SD card, #153
bazel build //cpu/vreteno/rust:ethperf_ram_bin //eth/hil:run   # Ethernet throughput, #1038
```

A Vivado build that the memory watchdog can reach is killed partway, so run each one detached, with `nohup` and `disown`, and poll its log.
Section 10 records what the last build of each produced, so that a bitstream built on the day can be compared with it.

## 2. The connection

The board is cabled to the machine that `.bazelrc` names in `TXHDL_BOARD_SERVER`.
Every target below reaches it over ssh, so the only thing to start by hand is the JTAG tunnel, and it is started first, in a terminal of its own, before any programming target (#392):

```sh
bazel run //cpu/vreteno/board/remote:hw_server     # leave it running
```

Every programming target then takes the same two arguments, kept in an array so the device pattern reaches Vivado unexpanded:

```sh
PROG=(--hostport localhost:3122 --device '*/xilinx_tcf/Digilent/*')
```

The serial line is watched with `bazel run //cpu/vreteno/board/remote:serial -- --seconds=60`.
A program is sent down it with `bazel run //cpu/vreteno/board/remote:load -- --image=$PWD/<image> --seconds=<n>`, which prints what the board answers for that long.
`--reset` on the loader resets the core over the serial line first, so a second program needs no reprogram.

Keep every transcript: run each command through `tee` into a file named for the check, `board-<issue>-<step>.log`, and attach it to the issue.

## 3. The order of the session

The checks share bitstreams, so this order programs the part four times and writes the flash once.

1. `vreteno_board_jtag_prog`, then the debug module probe (section 4).
2. `vreteno_board_boot_prog`, then the fence (section 5) and the entropy source (section 6).
3. `flagship_prog`, then fastboot (section 7).
4. `vreteno_board_prog`, then the DDR3 test from JTAG, then from the flash (section 8).
5. `flagship_flash`, which puts the flagship back in the flash, where it lives.

## 4. The debug module, #154

Towards #154: this is step 2's board proof; step 3's, OpenOCD through the transport, follows it.

```sh
bazel run //cpu/vreteno:vreteno_board_jtag_prog -- "${PROG[@]}"
bazel run //cpu/vreteno:vreteno_board_dm_probe 2>&1 | tee board-154-dm.log
```

Pass: every line that begins `dm:` ends in `ok` or gives a value, none says `BAD`, and the last is `dm: halt, registers, resume: every step ok`.
Capture: `board-154-dm.log`, with `grep '^dm:'` of it pasted into the issue.
It closes nothing; #154 stays open for step 3.

### Step 3: a stock OpenOCD through the transport

Every top has the debug transport behind `BSCANE2` on `USER4` now, so this runs on the boot bitstream of section 5, before its loads.
OpenOCD takes the cable itself, so the JTAG tunnel's `hw_server` is stopped first, and started again after for the sections that program.
The server starts it under the dynamic loader, as `ld.so ... bin/hw_server -stcp ...`, so its process name is `ld.so` and `pkill -x hw_server` finds nothing; the command line is matched instead (#895).
The bracket in the pattern keeps it from matching the remote shell that carries it, which `pkill -f` would otherwise kill, and its output with it.
The server has OpenOCD 0.12.0 and the Digilent configurations for the cable's FT232H (`0403:6014`); no `sudo` and no udev rule is needed.
The cable and the core are set up by `tools/openocd/ax7a200.cfg`, copied to the server first, which `//tools/openocd:ax7a200_cfg_test` reads with the pinned OpenOCD.
It names the cable `Digilent USB Device`, as this board's FT232H enumerates; the HS2 file alone looks for `Digilent Adept USB Device` and finds no device.

```sh
bazel run //cpu/vreteno:vreteno_board_boot_prog -- "${PROG[@]}"
ssh $TXHDL_BOARD_SERVER "pkill -f 'bin/[h]w_server -stcp'"
scp tools/openocd/ax7a200.cfg $TXHDL_BOARD_SERVER:/tmp/txhdl-ax7a200.cfg
ssh $TXHDL_BOARD_SERVER openocd -f /tmp/txhdl-ax7a200.cfg -c "'\
  init; halt; \
  echo \"state [xc7.cpu curstate]\"; echo [capture {reg pc}]; \
  echo [capture {mdw 0x1000 4}]; step; echo \"stepped [capture {reg pc}]\"; \
  resume; echo \"state [xc7.cpu curstate]\"; shutdown'" 2>&1 | tee board-154-openocd.log
```

Pass: `tap/device found: 0x13636093`, `Examined RISC-V core; found 1 harts` with `XLEN=32` and `misa=0x40141105` (RV32IMAC with S and U since #1012; `0x40001105` before it and `0x40001104` before #1010), `state halted`, a `pc`, the four words, a `stepped pc` one instruction on, and `state running`, which is the session `//cpu/vreteno:openocd_test` runs against the simulated board.
If OpenOCD says `no device found`, read the cable's product name on the server, `cat /sys/bus/usb/devices/*/product`, and set `ftdi device_desc` in the configuration to it.
Capture: `board-154-openocd.log`, posted on #154.

### Step 3, the gdb half: a program loaded and a breakpoint hit, #872

Same bitstream, after the OpenOCD session above has ended.
gdb runs here, from `//third_party/gdb`, and reaches the server's OpenOCD gdb port, 3333, over an SSH forward, as `hw_server`'s 3122 is forwarded; nothing is installed on the server.
OpenOCD binds its gdb port to the server's own loopback, so only the forward reaches it.

In one terminal, OpenOCD on the server with the forward, left running:

```sh
ssh -L 3333:localhost:3333 $TXHDL_BOARD_SERVER openocd -f /tmp/txhdl-ax7a200.cfg -c "'\
  bindto 127.0.0.1; gdb_port 3333; init'" 2>&1 | tee board-872-openocd.log
```

In another, gdb here, once OpenOCD has said `Listening on port 3333 for gdb connections`:

```sh
bazel run //third_party/gdb -- -nx -batch \
  -ex "set architecture riscv:rv32" -ex "set remotetimeout 60" \
  -ex "file $PWD/bazel-bin/cpu/vreteno/rust/gdbprobe_elf.elf" \
  -ex "target extended-remote localhost:3333" -ex load \
  -ex "break reached" -ex continue \
  -ex 'printf "a0 %d\n", $a0' -ex 'printf "SUM %d\n", *(unsigned int *)&SUM' \
  -ex "monitor shutdown" 2>&1 | tee board-872-gdb.log
```

Pass: `Loading section .text` at `0x40000000`, `Breakpoint 1, ... in reached ()`, `a0 45` and `SUM 45`, which is what `//cpu/vreteno:openocd_test`'s `gdb_loads_a_program_and_stops_at_a_breakpoint` prints against the simulated board.
gdb's last lines, a protocol error and `Remote connection closed`, are OpenOCD shutting down under it, and are expected.
`Failed to read memory` with `sysbus=skipped (unsupported size)` means the bitstream predates #873's bridge, which serves the 16-bit accesses gdb makes.
The program runs from the DDR3, so the controller must have calibrated, which the boot bitstream's third LED shows.
Capture: both logs, posted on #154, whose other done-when this is.

## 5. The loader's fence, #550 and PR 637

#550 is closed, and PR 637 merged; the board run of the new boot memory is what is owed, and its result goes on #550 as a comment.

```sh
bazel run //cpu/vreteno:vreteno_board_boot_prog -- "${PROG[@]}"
bazel run //cpu/vreteno/board/remote:load -- \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/hello_ram_bin.bin --seconds=10 \
    2>&1 | tee board-550-hello.log
```

Pass: the loader answers `mem` with the sum it read back from the memory, then `ok` and the address it loaded to, `40000000`, and then the program prints `hello from rust`.
`ok` comes only when the sum of the words that arrived matches, which is otherwise `bad sum`, and when the sum read back from the memory matches it too, which is otherwise `bad mem` (#1555).
Run it three times with `--reset`; the fence is about an order that a single run can get right by luck.
Capture: `board-550-hello.log`, and the bitstream's sha256 from section 10, since the fence is in the boot memory and so in the bitstream.

## 6. The entropy source, #458

Same bitstream as section 5; the source is on the peripheral page at `0x3500`.

```sh
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/trng_ram_bin.bin --seconds=30 \
    2>&1 | tee board-458-trng.log
bazel run //tools/trngstat -- $PWD/board-458-trng.log | tee board-458-stat.txt
```

The program prints `trng ok`, then `raw` and 4096 words, then `words` and 4096 words, then `end`, which is about seven seconds of serial line at 115200 baud.
Pass, first: `trng ok`, not `trng bad` nor `fault`, and the capture reaches `end`.
Pass, second: `trngstat` exits 0, which says the extractor's words are within the bounds it states: the bias within four standard deviations of a fair source, the correlation of bits inside a word within four of its own at every lag from one to eight, and at least 0.97 bits per bit of min-entropy by SP 800-90B's most common value estimate.
The raw words are measured and not judged, since the samples before the extractor are not expected to be fair; their correlations by lag, one to 31 inside a word, are what the issue asks for.
Capture: both files, and run the load twice more; then give all three captures to `trngstat` at once, which reports each and the three pooled, with one standard deviation beside every lag.
A bitstream that folds samples before the extractor is read with `-fold=2`, so that a lag of the words is also said in samples of the source (#794).
It closes #458 once the numbers are on it; the datasheet in #521 is written after them.

Done in part: extractor captures were taken on the board, and `docs/examples_sections/67_trng.tex` and `zephyr/drivers/entropy/vreteno_condition.h` report them, with the bias within its bound, about 0.99 bits per bit of min-entropy, and a correlation between neighbouring bits in every capture, at a lag that moves between builds.
So the second pass criterion is not met, and what is owed is the three-capture pooled report and the raw words' correlations on #458, then #521, and a board run of the conditioned output, `//zephyr:entropy`.

`trngstat` is a first estimate and not an SP 800-90B assessment: 4096 words is below the million samples the standard's full battery wants.
If the numbers are to be quoted as an assessment, the next step is a longer capture, which the program would need to be changed to print.
`trng_ram_bin` also prints samples in a row rather than windows of 32, which is what measures a period longer than a word, in two sections: `rawrun`, the run joined from overlapping windows a loop read (#805), and `rawcap`, the peripheral's own capture of 8192 samples (#817).
`trngstat` reads each out to `-maxlag` on its own, and never joins them, nor runs from different captures; the capture is the one to quote, since the joined run breaks wherever a read came late (#835).
A file with a heading twice is refused, so a log that holds two runs of the program must be split before it is read.

### The cycle counter, #807 and #848

Same bitstream again.
`steps_ram_bin` writes two back-to-back reads of `mcycle` into the data memory, which is block RAM and so has the same fetch every time, calls them eight times, and prints each difference.

```sh
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/steps_ram_bin.bin --seconds=10 \
    2>&1 | tee board-848-steps.log
```

Pass: eight lines of `mcycle step 13`.
That is what `board_test`'s `mcycle_steps_steadily_between_two_reads` pins in simulation; the core from before #807's fix printed a steady 12 there, a count lost to each read.
A steady number other than 13 is the board's fetch differing from the simulation's and is a finding to note on #848; a number that moves from line to line means the routine did not run from the data memory.
It closes #848 once the result is on it.

### The path into DDR3, #1023

Same bitstream again.
`ddr3bw_ram_bin` writes two routines into the data memory, sixteen loads and sixteen stores with a fence, and times each with `mcycle`, four times against the DDR3 and four times against the data memory.

```sh
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/ddr3bw_ram_bin.bin --seconds=10 \
    2>&1 | tee board-1023-ddr3bw.log
```

Pass: sixteen lines, four each of `bw load ddr3`, `bw load dmem`, `bw store ddr3` and `bw store dmem`, each four steady.
In simulation, `board_test`'s `the_ddr3_path_is_timed_by_the_core` pins 461, 397, 251 and 250: the DDR3's loads cost four cycles a word more than the data memory's, the bridge's own, since the controller's model answers in one cycle.
On the board, the loads' difference divided by sixteen, less those four, is what AMD's controller adds to a word, and that is the number #1023 wants.
The table `bazel run //ddr3:bw` prints then gives the path's throughput near that latency, since a word costs the latency and four cycles.
The stores are expected to differ little, as in simulation: the core fetches each store over the bus slower than the path takes it, so they say the core cannot fill the path, not how fast the path is.
Post the log on #1023.

Done on October 4, 2026, by hil, on the flagship: sixteen loads took 841, 837, 834 and 826 cycles against the DDR3 and 397 against the data memory, about 27 cycles a word more, so the controller adds about 23.
The result and what it decides are in `docs/ddr3-throughput.md`, section 6, and on #1023.

### The instruction cache's CPI, #1320

Same bitstream again.
`cpi_ram_bin` writes `icache_test`'s loop, three instructions a thousand times round, into the data memory and into the DDR3, and times it from each with `mcycle` and `minstret`: cold, with a `fence.i` in the time that clears the tags and leaves the line to be filled, and warm, straight after.

```sh
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/cpi_ram_bin.bin --seconds=10 \
    2>&1 | tee board-1320-cpi.log
```

Pass: four lines, `cpi dmem cold`, `cpi dmem warm`, `cpi ddr3 cold` and `cpi ddr3 warm`, each with its cycles and 3004 instructions.
In simulation, `board_test`'s `the_cache_loop_is_timed_by_the_core` gives 8077, 7036, 8149 and 7084 cycles: warm, 2.34 cycles an instruction from either memory, and cold about 1040 more, the 1024 cycles the tags take to clear and the line's fill.
The warm figures are the steady CPI #1306 left unmeasured; a warm DDR3 figure far above the data memory's would mean the line is not staying in the cache, and is a finding.
The cold DDR3 figure less the warm one, less 1024, is what a line's fill from AMD's controller costs.
Post the log on #1320.

### The DDR3's writes and strobes, #1174

Same bitstream again, or any flagship session, a Linux one included: the program leaves alone what the flagship keeps in the memory.
`ddr3ram_ram_bin` writes and reads back a block of words from `0x4100_2000`, words up to the top of the gigabyte from `0x4400_0000`, and bytes and halfwords under their strobes, and reads the bytes and halfwords back at every offset.
It keeps clear of the loaded program, the Ethernet slots at `0x4100_0000`, the device tree at `0x4100_8000` and Razboj's frame buffer at `0x4200_0000`.

```sh
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/ddr3ram_ram_bin.bin --seconds=10 \
    2>&1 | tee board-1174-ddr3ram.log
```

Pass: `ddr3ram words ok`, `ddr3ram high ok`, `ddr3ram strobes ok`, `ddr3ram lanes ok` and `ddr3ram ok`, and the core halts, which lights the first LED.
`board_test`'s `the_loaded_memory_test_passes_through_the_model` pins the same lines in simulation.
A check that fails says how many accesses were wrong, the first wrong address and what was read there, and the core spins; post the log on #1174 and #1023.

## 7. Fastboot, #143

The flagship built on September 26 missed timing (section 10); it has met it since #753 closed (9777b4a9).

The fastboot server is the Zephyr program `//zephyr:fastboot`, sent down the serial line by the flagship's loader; it takes the address `192.168.1.50` on the board's Ethernet port.
The port is cabled to the board server's interface `fpga-a200t-eth0`, so the fastboot client runs there, over ssh, and that interface wants an address on the same network, once:

```sh
ssh $TXHDL_BOARD_SERVER sudo ip addr add 192.168.1.1/24 dev fpga-a200t-eth0
ssh $TXHDL_BOARD_SERVER sudo ip link set fpga-a200t-eth0 up
ssh $TXHDL_BOARD_SERVER mkdir -p txhdl_fastboot
scp "$(bazel cquery --output=files @multitool//tools/fastboot 2>/dev/null)" \
    bazel-bin/cpu/vreteno/rust/hello_fastboot.bin $TXHDL_BOARD_SERVER:txhdl_fastboot/
```

Then, with the flagship in the part:

```sh
bazel run //flagship:flagship_prog -- "${PROG[@]}"
bazel run //cpu/vreteno/board/remote:load -- \
    --image=$PWD/bazel-bin/zephyr/fastboot.bin --seconds=60 \
    2>&1 | tee board-143-listen.log
ssh $TXHDL_BOARD_SERVER txhdl_fastboot/fastboot -s tcp:192.168.1.50 \
    getvar max-download-size 2>&1 | tee board-143-getvar.log
bazel run //cpu/vreteno/board/remote:serial -- --seconds=30 \
    2>&1 | tee board-143-boot.log &
ssh $TXHDL_BOARD_SERVER txhdl_fastboot/fastboot -s tcp:192.168.1.50 \
    boot txhdl_fastboot/hello_fastboot.bin
```

The three checks #143 lists, in its comment on PR #528:

1. The console says `fastboot: listening on port 5554`.
2. `getvar max-download-size` answers `0x00fff000`.
3. `fastboot boot` of `hello_fastboot` ends with `hello from rust` on the serial line, which proves the copy, the jump and the posted stores read back on real DDR3.

Done on September 28 and again on October 1, 2026: all three passed, on flagship `9755f9c85be5d066` and on main `b747d42`'s flagship; both runs are on #143 and in `zephyr/README.md`.

`hello_fastboot` is `hello_ram_bin` padded with zeros to 4096 bytes.
Stock `fastboot` refuses the 148 bytes of `hello_ram_bin` as `too short`, before it sends anything, because it reads a boot image header's worth of a file first (#799); `//zephyr:fastboot_test` checks both off the board.

The fastboot server is 28959 words, and at one acknowledgement a word it takes about 90 seconds to send.
`load` counts `--seconds` from the end of the transfer and does not cut a transfer that is still moving, so the 60 are for watching alone; fastboot said `listening` about 15 seconds after its transfer ended (#784).
`load` says how far it has got every 4096 words, and a transfer that stops says at which word and fails.
The serial watcher and the loader both hold the serial port, so the watcher in step 3 starts after the loader's `--seconds` have run out, or the loader's own output, which runs that long, is read for `hello from rust` instead.
If the ping to `192.168.1.50` from the board server fails, the link is the first thing to look at: `ip link show fpga-a200t-eth0` should say `LOWER_UP`.
Capture: the three logs.
It closes #143 when all three pass.

### The PHY's registers, #864

Same bitstream as the fastboot check, the flagship built from PR 863 or after it, which shifts the receive clock 78.75 degrees and brings the JL2121's MDIO out to the core.
The program reads the PHY's 32 management registers over MDIO and prints them; it writes nothing to the PHY, not even a page select.

```sh
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/phyregs_ram_bin.bin --seconds=10 \
    2>&1 | tee board-864-phyregs.log
```

Pass: `phy at` and an address, then eight lines from `r00` to `r28`, four registers of four hexadecimal digits each.
Registers 2 and 3, the last two words of the `r00` line, are the PHY's identifier; they should not both be `0000` or `ffff`.
`no phy` means nothing answered at any of the 32 addresses: the pins, the pull-up or the PHY's reset are the first things to look at.
`//cpu/vreteno:board_test`'s `the_phy_registers_read_on_the_board` runs the program against a model PHY holding the registers the board answered with, so a later run that differs from it beyond the two bits clause 22 clears on a read, link status in register 1 and page received in register 6, is a change on the board or in the program.

Done on October 3, 2026, by txhdl-hil: the JL2121 answered at address 0 with the identifier `937c 4032`, and two runs agreed except for two bits clause 22 clears on a read. Both dumps are on #864.
The register that holds the receive delay, by JLSemi's Linux driver, is on page 3336, and this program reads only page 0, so the dump cannot show it; reading that page means writing the page register, which waits on the user.

### The PHY's delay bits, #869

Same bitstream.
**This program writes the PHY**: register 31, the page select, twice, to select page 3336 and then to put back the page it found.
It writes nothing else, and nothing at all if the first read of register 31 comes back `ffff`.
The Overseer relayed the user's approval of these two writes on October 3, 2026; confirm it with the user before the run.

```sh
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/phydelay_ram_bin.bin --seconds=10 \
    2>&1 | tee board-869-phydelay.log
```

Pass: `phy at 0`, `page was 0000`, `p3336 r17` and a word, `tx delay` and `rx delay` each 0 or 1, and `page now` with the same word as `page was`.
`page not restored` is a fail; the next step is then a power cycle of the board, which puts the PHY back as it powers up.
`page read failed, nothing written` means register 31 did not answer, and the PHY was left alone.
`rx delay 1` says the JL2121 delays its receive clock 2 ns itself, which is what #864's measurements predict; `rx delay 0` says the centring comes from somewhere else, and #864's documents are wrong in that part.
`tx delay 1` is the prediction for transmit, since the transmit clock leaves the FPGA edge-aligned and the link is clean (#231); a 0 there would mean the transmit centring, too, comes from somewhere other than this bit.
`//cpu/vreteno:board_test`'s `the_phy_delay_bits_read_through_the_page` runs the program against the model, whose word is its own (`0200`), and checks that the frames are exactly read, write, read, write, read, and that the page select is left as found; `a_failed_page_read_writes_nothing` checks the abort.
Put the log on #869.

### The SD card, #153

The boot bitstream, programmed over JTAG, built from the change that brings the slot's pins out (bank 16, E13 for the clock, E14 for the command line, D15, D14, F14 and F13 for the data lines), with a card in the slot, J7.
Nothing is written to flash, and the program writes nothing to the card: it sends no write command.

```sh
bazel run //cpu/vreteno:vreteno_board_boot_prog -- "${PROG[@]}"
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/sdprobe_ram_bin.bin --seconds=10 \
    2>&1 | tee board-153-sdprobe.log
```

Pass, in this order:
* `cmd8 000001aa`.
* `ocr` and a word with its top bit set, then `sdhc` for a card above 2 GB.
* `cid` and four words, the manufacturer's identifier first, then `rca` and the card's address.
* `blk0` and 32 lines of 16 bytes.
* `wide same`, which says that the block read on four lines is the one read on one.
* `boot 55aa` for a card with a partition table, `boot none` otherwise.

`no card` means nothing answered `CMD8`: either there is no card, or the clock or the command line is not reaching it.
`never ready` means the card answered but did not finish powering up within about a second.
`cmd` and a number with `failed` names the command whose response timed out or failed its CRC.
`wide differs` with a sound `blk0` points at `sd_dat[1]` to `sd_dat[3]`, which only the four-line read uses.
`//cpu/vreteno:board_test`'s `the_sd_card_is_read_on_the_board` runs the program against the model card and checks the printout and that the card's blocks are unchanged.
Put the log on #153; it closes #153 when it passes.

Done on October 3, 2026, by txhdl-hil, twice, the two runs agreeing line for line: a 32 GB card answered `cmd8 000001aa` and `ocr c0ff8000 sdhc`, with the CID `19445941 53544300 0000003a 9c00ec7b`.
Its block 0 is a partition table with one FAT32 partition from sector 8192, 0x03b84000 sectors long, and the run ended `wide same` and `boot 55aa`.
Both logs are on #153.

### The SD card through memory, #912

The same as for #153 above, from the bitstream of #912's change, which carries the SD host's two engines, with the same card in the slot.
`sdprobe` now ends with one more line: blocks 0 to 3 read with one `CMD18` straight into the DDR3, the card sending them back to back, then the same four blocks read one at a time through the buffer and compared word by word.
Nothing is written to the card.

Pass: the six lines above, unchanged, then `dma same`.
`dma differs` and a number names the first block whose words in memory are not the ones the one-block read gave.
A `cmd18 failed` or `cmd12 failed` line means the multi-block read itself went wrong, before any comparison.
`//cpu/vreteno:board_test`'s `the_sd_card_is_read_on_the_board` runs the same program against the model card and expects `dma same`.
Put the log on #912; it closes #912 when it passes.

### The video scanout under load, #151

The flagship from the change that joins the scanout to the board (#151's second pull request), programmed over JTAG, with a monitor on the HDMI connector and the Ethernet cable in, as for fastboot above.
Nothing is written to flash.

```sh
bazel run //flagship:flagship_prog -- "${PROG[@]}"
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/scanprobe_ram_bin.bin --seconds=25 \
    2>&1 | tee board-151-scan.log
```

The program paints a 640 by 480 frame into the DDR3 and shows it through the scanout.
It reads the underflow bit after two seconds on an idle bus.
Then it reads it again after ten seconds in which the core copies a quarter of a megabyte back and forth in the DDR3 and the Ethernet port sends a full frame whenever it is ready.

Pass, in this order:
* `scan probe`.
* `scan idle 0`.
* `scan load 0`, with the copies and frames counted, both above zero.
* `scan ok`.
* On the monitor: eight colour bars over a grey ramp, the TxHDL logo in the bottom right corner, steady, with no torn or repeated lines.

`scan idle 1` means a line came late with nothing else on the bus: the fetch or the crossing is wrong, not the bandwidth.
`scan load 1` with `scan idle 0` means the scanout's share of the bus, a turn in six under this load (`docs/vreteno.tex`), is not enough: the case for giving the scanout priority, or for more than one access in flight to the DDR3 (#1023).
`frames 0` means the port never became ready, so the load was the copy alone; say so with the result.
A picture with the bars but no logo, or bars of the wrong colours, points at the pixel format, `0x00RRGGBB`.
Put the log, and a photograph of the screen, on #151.

### What a full frame costs to update, #151

The same flagship and setup as the scanout check above.
Nothing is written to flash.

```sh
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/fbtime_ram_bin.bin --seconds=10 \
    2>&1 | tee board-151-fbtime.log
```

The program times two ways of updating the whole screen with the cycle counter.
It paints the video peripheral's 160 by 120 framebuffer, a register write a pixel.
Then it paints a 640 by 480 frame into the DDR3 and switches the scanout to it.

Pass, in this order:
* `fb time`.
* `fb regs` with three numbers: the cycles, the cycles a pixel in hundredths, and a 640 by 480 screen's cycles at that rate.
* `fb ddr3` with the same three.
* `fb done`.
* On the monitor, after `fb ddr3`: a pattern of colour ramps over the whole screen.

The comparison #151 asks for is the third number of each line: a full screen through the peripheral's registers against a full screen through the DDR3.
Put the log on #151.

### Razboj on the board, #985 and #1169

The flagship from a `main` that holds #985 (PR 1160), programmed over JTAG, with a monitor on the HDMI connector and the Ethernet cable in, as for the scanout above.
Nothing is written to flash.
It needs no order against the scanout check: each program sets the scanout's base itself.

```sh
bazel run //flagship:flagship_prog -- "${PROG[@]}"
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/razprobe_ram_bin.bin --seconds=10 \
    2>&1 | tee board-985-razboj.log
```

The program shows the scanout from Razboj's frame at `0x4200_0000`.
It writes a display list of four entries at `0x4280_0000` and rings the doorbell at `0x3900`.
It waits for the count to read zero and for the rasteriser to say it is idle, then reads six pixels back over the bus.

Pass, in this order:
* `razboj probe`.
* `razboj rung`.
* `razboj idle` and a number of cycles: the whole list, a full-screen backdrop and three shapes.
* `razboj ok`.
* On the monitor: a dark blue screen with a red rectangle at the top left, a green triangle pointing down at the top right, and a sky blue triangle pointing up at the bottom middle, steady.

`razboj stuck`, with the count and the idle bit, means the list was never finished: a count still set means the rasteriser never drew it or never wrote the zero back, and a zero with idle 0 means a write it made was never answered.
`razboj bad`, with a column, a row and the word read, means the rasteriser drew, but not what the list says; the word read says whether the pixel was left alone, drawn in the wrong colour, or drawn at the wrong stride.
A right serial log with a wrong picture points at the scanout's stride of 4096 rather than at Razboj.
Put the log, the cycle count, and a photograph of the screen on #985.

### Linux on the Ethernet port, #1203

A flagship built from a `main` that holds #1203, since the port's registers moved to LiteX's offsets in the hardware.
First run the Ethernet transmit and echo checks above from that `main`, exactly as written: a failure there is the register move, not Linux.

Then boot `//linux:board_boot` from the same `main` with fastboot, as for M12.
The kernel says `liteeth 3400.ethernet eth0: irq 2 slots: tx 2 rx 2 size 2048`.
The board's image does not start the network itself, since `/init` does that only for `txhdl.net` on the command line, so from the board's shell:

```sh
ip addr add 192.168.1.50/24 dev eth0
ip link set eth0 up
ping -c 3 192.168.1.1
```

Then from srv, which needs no `sudo`, `ping -c 3 192.168.1.50`, and on the board `ip -s link show eth0`.
The cable is point to point to srv's `fpga-a200t-eth0`, which already holds 192.168.1.1, so there is nothing to configure there.

Pass: three replies each way, and receive and transmit counts on `eth0` that are not zero.
No `liteeth` line means the kernel found no port in the tree, or found it and failed to map it.
A probe with no replies, and transmit counts going up, means frames leave and nothing comes back: look at srv's side with `tcpdump -i fpga-a200t-eth0`.
Put the log on #1203.

### The GL icosahedron's fault, #1214

`ico_gl_hdmi` took a load access fault once, at its first read of Razboj's doorbell, on a bitstream where `ico_hdmi` ran for minutes.
Both programs now report a fault themselves, with `trap::say_faults` from the HAL, on one line:

```
trap <cause> at <mepc> mtval <mtval> a0 <a0> ra <ra>
```

Each is eight hexadecimal digits: the cause, the instruction, `mtval`, which for a refused load or store is the address, and `a0` and `ra` as the faulting code had them.
The loader's own `trap` line has only the first two, since by the time it runs every register is its own.
`//cpu/vreteno:board_test` checks the report on a load from a hole in the map.

Any flagship from #985 on, programmed over JTAG, with a monitor on the HDMI connector.
The order matters, so that the first run follows a reset with nothing else run before it:

```sh
bazel build //cpu/vreteno/rust:ico_gl_hdmi_bin //cpu/vreteno/rust:ico_hdmi_bin
bazel run //flagship:flagship_prog -- "${PROG[@]}"
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/ico_gl_hdmi_bin.bin --seconds=20 \
    2>&1 | tee board-1214-first.log
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/ico_hdmi_bin.bin --seconds=20 \
    2>&1 | tee board-1214-hand.log
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/ico_gl_hdmi_bin.bin --seconds=20 \
    2>&1 | tee board-1214-again.log
```

Pass: each run says `ico 20 faces` and no `trap`, and the icosahedron turns on the monitor.
A `trap` line is the result #1214 wants, whichever run it comes in.
An `mtval` of `00003900` means the doorbell refused a read it should answer; any other address means the program read somewhere it did not mean to, and the address says where.
Which of the three runs trapped says whether what ran before matters.
Put the three logs on #1214.

### A program that stops answering after a serial reset, #1317

On main 41e5b497's flagship, `shadeprobe` once stopped after `shade rung`, with no `shade stuck` line, while the screen showed the scanout's magenta for 23 s until the next `load --reset`.
One serial reset in about twenty did it, and it has not happened since, so it is caught when it happens rather than run on purpose.

If a program falls silent and the screen is solid magenta, or a load prints nothing it should have, **load nothing else and reset nothing**: the state that says why is lost with the next reset.
Stop the serial watcher by its pid on the server, then stop `hw_server` as in section 4, step 3, and read the machine with OpenOCD in this order:

```sh
ssh $TXHDL_BOARD_SERVER "pkill -f 'bin/[h]w_server -stcp'"
scp tools/openocd/ax7a200.cfg $TXHDL_BOARD_SERVER:/tmp/txhdl-ax7a200.cfg
ssh $TXHDL_BOARD_SERVER openocd -f /tmp/txhdl-ax7a200.cfg -c "'\
  riscv set_command_timeout_sec 10; init; halt 5000; \
  echo \"state [xc7.cpu curstate]\"; echo [capture {reg pc}]; \
  echo [capture {reg mepc}]; echo [capture {reg mcause}]; \
  riscv set_mem_access sysbus; \
  echo \"scan status [capture {mdw 0x3288}]\"; \
  echo \"scan stuck_at [capture {mdw 0x3290}]\"; \
  echo \"razboj count [capture {mdw 0x3900}]\"; \
  echo \"razboj status [capture {mdw 0x3904}]\"; \
  echo \"kept trap [capture {mdw 0x1000 5}]\"; \
  shutdown'" 2>&1 | tee board-1317-wedge.log
```

1. **Halt, and the program counter.** A halt that times out says the core is waiting on a bus access that never came back, which is the finding. A `pc` says where it stopped, and the instruction there names the load, its base register the address.
2. **The scanout's status and `stuck_at`**, `0x3288` and `0x3290` (`ScanCtl`'s words 2 and 4 at the slot's `0x3280`): bit 1 of the status set and an address in `stuck_at` say which line's fetch never came back.
3. **Razboj's count and status**, `0x3900` and `0x3904`: a count left non-zero with the idle bit clear says Razboj had a list in flight.
4. If the system bus reads in steps 2 and 3 time out as well, the bus itself is stuck, not one host.
5. **The trap the loader kept**, five words at `0x1000` (issue 1411): `54524150` ("TRAP"), a count, and the last trap's `mcause`, `mepc` and `mtval`. A `pc` in the loader, near zero, with a count that went up, says the program trapped and came back through `mtvec` 0, whatever `mcause` reads: the loader clears `mcause` and `mepc` once it has said them, which made the 2026-10-07 catch read as a jump to zero with no trap (#1317).

The 10 s command timeout is not a choice: with the default 2 s, the first session on 2026-10-07 failed with "DMI operation didn't complete" while the core waited on the stalled access.

Capture: `board-1317-wedge.log` and the program's serial log, posted on #1317, with the time it began so srv's recording can be matched to it.
Then reset and go on with the session.

### Gouraud shading on the board, #989

The flagship from any `main` that holds #985 (PR 1160), programmed over JTAG, with a monitor on the HDMI connector, as for Razboj above.
Nothing is written to flash, and it needs no order against the other Razboj checks: the program sets the scanout's base itself.

```sh
bazel build //cpu/vreteno/rust:shadeprobe_ram_bin
bazel run //flagship:flagship_prog -- "${PROG[@]}"
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/shadeprobe_ram_bin.bin --seconds=20 \
    2>&1 | tee board-989-shade.log
```

The program has Razboj draw a triangle with red, green and blue at its corners over a dark blue backdrop.
Then it reads every one of the 640 by 480 pixels back and compares each with what Razboj's model draws there, which `//cpu/vreteno/rust:shadeprobe_list_test` holds the program's expectation to, pixel for pixel.

Pass, in this order:
* `shade probe`.
* `shade rung`.
* `shade idle` and a number of cycles.
* `shade ok 104000`: every pixel as the model draws it, and the triangle's 104000 among them.
* On the monitor: a triangle pointing up, red at its top, green at its bottom right and blue at its bottom left, blending smoothly between them, on dark blue, steady.

`shade bad` gives the first wrong pixel's column, row, the word read and the word expected, and `shade wrong` how many were wrong.
A few wrong pixels along an edge point at the edge rule rather than at the shading; wrong colours inside point at the planes' steps.
`shade stuck` means the list was never finished, as `razboj stuck` above.
Put the log, the cycle count, and a photograph of the screen on #989, which a pass finishes.

### EGL on the board, #996

The flagship from a `main` that holds #996's board half, programmed over JTAG, with a monitor on the HDMI connector, as for Razboj above.
Nothing is written to flash.
Two programs, in this order: the first is the swap alone, and the second is the whole of GL and EGL under Zephyr, so a failure of the second with the first passing is in GL or Zephyr and not in the hardware.

```sh
bazel build //cpu/vreteno/rust:eglboard_ram_bin //zephyr:gles
bazel run //flagship:flagship_prog -- "${PROG[@]}"
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/eglboard_ram_bin.bin --seconds=10 \
    2>&1 | tee board-996-swap.log
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/zephyr/gles.bin --seconds=60 \
    2>&1 | tee board-996-gles.log
```

`eglboard` writes one rectangle into the second buffer through the board's machine, has Razboj draw it, shows the second buffer and waits for the vertical blanking, as `eglSwapBuffers` does.

Pass, for the first:
* `inside ff005aa5`: the pixel Razboj drew.
* `base 42200000`: the scanout's base, row 512 of the framebuffer.
* `egl board ok`.
* On the monitor: a speck of blue a few pixels across near the top left, over whatever the DDR3 held.

`egl board bad` prints what it read.
An `inside` of anything else means Razboj did not draw the list; a run that stops before `inside` means the doorbell never went back to zero or the blanking never came, which `razprobe` above tells apart.

The Zephyr program is about 107 KB, so its transfer takes about a minute and a half, as fastboot's does (section 7).

Pass, for the second:
* `gles egl 1.4`, once.
* `gles frame 0 cycles` and a number, then a line every sixty frames, the frame count rising by 60.
* On the monitor: a gold fan of six triangles on dark blue, lit from the eye, turning steadily, with no torn or half-drawn frame and nothing flickering at its edges.

`gles egl failed` or `gles egl setup failed` with a number is an EGL error, `0x3000` and up, and names the call that failed.
A console that says `gles egl 1.4` and nothing more means the first swap never returned: the draw or the blanking hung, which the first program checks.
A picture that tears means the show was not at the blanking; one that flickers between two pictures means a buffer was shown while it was drawn.
The cycles a frame takes, against `ico_gl_hdmi`'s, are what the issue asks to be measured; a frame waits for one blanking, so the frame rate is at most the screen's.
Put both logs, the cycle counts, and a photograph or a short video of the screen on #996.

### Razboj's tile buffer on the board, #1255

A flagship from the change that adds the tile buffer (#1255's pull request, or a `main` that holds it), programmed over JTAG, with a monitor on the HDMI connector.
Nothing is written to flash.

The change leaves a flat list drawing as before, so the check is that the three Razboj programs that draw flat lists still draw what they drew, on a bitstream whose Razboj also has the tile buffer:

```sh
bazel build //cpu/vreteno/rust:razprobe_ram_bin //cpu/vreteno/rust:shadeprobe_ram_bin \
    //cpu/vreteno/rust:ico_hdmi_bin
bazel run //flagship:flagship_prog -- "${PROG[@]}"
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/razprobe_ram_bin.bin --seconds=10 \
    2>&1 | tee board-1255-razprobe.log
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/shadeprobe_ram_bin.bin --seconds=20 \
    2>&1 | tee board-1255-shade.log
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/ico_hdmi_bin.bin --seconds=30 \
    2>&1 | tee board-1255-ico.log
```

Pass:
* `razboj ok`, as under "Razboj on the board, #985 and #1169" above.
* `shade ok 104000`, as under "Gouraud shading on the board, #989" above.
* `ico 20 faces` and the icosahedron turning, with no `trap`.

A program that draws a tile table on the board is not in this check; `//cpu/vreteno:board_test`'s `razboj_draws_a_list_in_tiles` draws one through the board's model.
Put the three logs on #1255.

### Depth on the board, #992 and #1273

A flagship with Razboj's depth test, from #992's pull request or a `main` that holds it, programmed over JTAG, with a monitor on the HDMI connector.
Nothing is written to flash.

`ico_gl_hdmi` now hides the icosahedron's back faces with the depth test rather than culling them, and draws each frame as a tile table, the first program to draw one on the board.
`ico_hdmi`, the hand-written list, still culls, and draws flat:

```sh
bazel build //cpu/vreteno/rust:ico_gl_hdmi_bin //cpu/vreteno/rust:ico_hdmi_bin
bazel run //flagship:flagship_prog -- "${PROG[@]}"
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/ico_gl_hdmi_bin.bin --seconds=30 \
    2>&1 | tee board-1273-depth.log
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/ico_hdmi_bin.bin --seconds=30 \
    2>&1 | tee board-1273-hand.log
```

Pass:
* Each run says `ico 20 faces`, then its cycles lines, and no `trap` and no `ico bin refused`.
* Through GL the icosahedron turns with no back face showing through a front one, and looks as the hand-written one does; `//cpu/vreteno/rust:ico_gl_test` finds the two pictures differ only on the outline.
* The logo stays in its corner, since a tile's write-out leaves the pixels no entry wrote.

The `draw` count of `ico gl list` now includes the tile buffer's write-out; set it beside `ico razboj list`'s.
Put both logs on #1273.

### The textured icosahedron, #997

A flagship with Razboj's texture cache, from a `main` that holds #1391 (2b8ee847 or later), programmed over JTAG, with a monitor on the HDMI connector.
On an older flagship Razboj passes over the texture slots and the faces come out plain.
Nothing is written to flash.

`ico_tex_hdmi` is `ico_gl_hdmi` with each face textured: a checker of 32 by 32 texels, white and dark teal, at the nearest texel, twice across each face, the lighting's colour modulating it.
Its texture's room is two megabytes into Razboj's list memory, and each frame uploads the texture there and draws as a tile table:

```sh
bazel build //cpu/vreteno/rust:ico_tex_hdmi_bin
bazel run //flagship:flagship_prog -- "${PROG[@]}"
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/ico_tex_hdmi_bin.bin --seconds=60 \
    2>&1 | tee board-997-tex.log
```

Pass:
* The run says `ico 20 faces`, then `ico gl tex list` cycles lines, and no `trap` and no `ico bin refused`.
* The icosahedron turns with the checker on every face, straight in perspective, and no back face shows through a front one.
* A still of the recording looks as `bazel run //cpu/vreteno/rust:ico_tex_frame -- <n> $PWD/f.png` draws frame `n` through Razboj's model; `//cpu/vreteno/rust:ico_gl_test` holds the rasteriser's own frame to the model's, byte for byte.

Through GL a frame's list is built and binned while Razboj draws the frame before (#1433), so each line says `list`, the core's building and binning; `wait`, what was left of the frame before's drawing; and `frame`, the whole frame with the wait for the blanking. The frame rate comes in steps of blankings, 1.667 M cycles at 60 Hz.
Set the counts beside `ico gl list`'s, and put the log and the recording on #997.

### The mipmapped icosahedron, #997 step 4

A flagship from a `main` that holds PR 1471 (#997 step 3), which filters and mipmaps in the rasteriser, programmed over JTAG, with a monitor on the HDMI connector.
On an older flagship the rasteriser reads the base level at the nearest texel, and the floor's far rows shimmer instead of fading to grey.
Nothing is written to flash.

`ico_mip_hdmi` is `ico_tex_hdmi` built with `--cfg=mip`.
The checker is filtered with `GL_LINEAR_MIPMAP_LINEAR` and `GL_LINEAR`, and its levels are generated as it is uploaded.
A floor recedes below the solid, rows 296 to 400 from the left edge to column 480, clear of the logo's columns, with the checker sixteen times along it, so its far rows read the levels below the base.

```sh
bazel build //cpu/vreteno/rust:ico_mip_hdmi_bin
bazel run //flagship:flagship_prog -- "${PROG[@]}"
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/ico_mip_hdmi_bin.bin --seconds=60 \
    2>&1 | tee board-997-mip.log
```

Pass:
* The run says `ico 20 faces`, then `ico gl mip list` cycles lines, and no `trap` and no `ico bin refused`.
* The floor's checker is sharp near the bottom and fades smoothly to an even colour toward the far edge, with no shimmer as the solid turns; the solid's faces are filtered rather than blocky.
* A still of the recording looks as `bazel run //cpu/vreteno/rust:ico_mip_frame -- <n> $PWD/f.png` draws frame `n` through Razboj's model; `//cpu/vreteno/rust:ico_mip_test` holds the rasteriser's own frame to the model's, byte for byte.

Set the cycles lines beside `ico gl tex list`'s, and put the log and the recording on #997.

### Doom, #1177

A flagship from a `main` that holds #1445 (the data cache) or later, programmed over JTAG, with a monitor on the HDMI connector, and the board's serial port open on the board server so that keys can be typed into it.
Nothing is written to flash.

`//demo/doom:doom` is doomgeneric for the core, bare metal, with Freedoom's first episode as its game data (#1177).
It is 314 KB, too large for the loader's serial line, so it goes through fastboot (section 7), which stages 48 MiB since #1177, and the game data is 28.8 MB.
The game data goes as the boot image's ramdisk, and the program reads it where fastboot staged it, at `0x4800_0000`:

```sh
bazel build //zephyr:fastboot //demo/doom:doom @freedoom//:freedoom1.wad
scp bazel-bin/demo/doom/doom.bin \
    "$(bazel info output_base)/external/+http_archive+freedoom/freedoom1.wad" \
    $TXHDL_BOARD_SERVER:txhdl_fastboot/
bazel run //flagship:flagship_prog -- "${PROG[@]}"
bazel run //cpu/vreteno/board/remote:load -- \
    --image=$PWD/bazel-bin/zephyr/fastboot.bin --seconds=60 \
    2>&1 | tee board-1177-listen.log
ssh $TXHDL_BOARD_SERVER txhdl_fastboot/fastboot -s tcp:192.168.1.50 \
    getvar max-download-size
bazel run //cpu/vreteno/board/remote:serial -- --seconds=300 \
    2>&1 | tee board-1177.log &
ssh $TXHDL_BOARD_SERVER txhdl_fastboot/fastboot -s tcp:192.168.1.50 \
    boot txhdl_fastboot/doom.bin txhdl_fastboot/freedoom1.wad
```

Keys are bytes on the serial line, each held for 180 ms: `w`, `a`, `s` and `d` or the arrows to move, `f` to fire, the space bar to use, Enter, Escape, `y`, `n` and the digits.
Enter three times from the title goes through the menu, the episode and the skill, and a fourth starts E1M1.

Pass:
* `max-download-size` answers `0x02fff000`.
* The serial log has Doom's startup text, ending in `I_InitGraphics`, then a `doom frame` line every 64 frames, with the cycles of the last frame and the frames a second.
* The screen shows the title and the demo loop, 320 by 200 doubled, between black bands, then E1M1 after the keys, and the view moves with them.

Done on October 8, 2026, on flagship main ed9a9a70 with #1445, with a recording: the demo loop at 4.6 to 6.0 frames a second, E1M1 at 6.3 to 7.5, the 28.4 MB boot in 31.9 s.
E1M1 started 5 s after the Enter that chose the skill, and the keys moved, turned and fired.
The logs are on #1177.

### The ES 1.1 conformance suite, #999

#999's suite, the same C programs `//gles:conform_test` runs on the host against Razboj's model, run on the board through GL, EGL and the board's machine, so that the board's results can be set beside the model's.
A flagship from a `main` that holds the GL stack through #999, programmed over JTAG, with the board's serial port open on the board server.
Nothing is written to flash.

`//gles:conform_board` is the suite for the core, bare metal, 167 KB, sent through fastboot (section 7):

```sh
bazel build //zephyr:fastboot //gles:conform_board
scp bazel-bin/gles/conform_board.bin $TXHDL_BOARD_SERVER:txhdl_fastboot/
bazel run //flagship:flagship_prog -- "${PROG[@]}"
bazel run //cpu/vreteno/board/remote:load -- \
    --image=$PWD/bazel-bin/zephyr/fastboot.bin --seconds=60 \
    2>&1 | tee board-999-listen.log
bazel run //cpu/vreteno/board/remote:serial -- --seconds=600 \
    2>&1 | tee board-999.log &
ssh $TXHDL_BOARD_SERVER txhdl_fastboot/fastboot -s tcp:192.168.1.50 \
    boot txhdl_fastboot/conform_board.bin
```

Pass:
* The serial log has a line a check, `PASS` or `FAIL` with its group and name, and ends with the count, `DONE <passed> passed, <failed> failed`.
* The log's failures are the host's, which `gles/conform/known_failures.txt` lists, or each difference has an issue: the board draws with Razboj itself and reads back from the DDR3, where the host draws with its model.
  `comm -3 <(tr -d '\r' < board-999.log | sed -n 's/^FAIL \([^:]*\):.*/\1/p' | sort) <(grep -v '^#' gles/conform/known_failures.txt | awk 'NF {print $1}' | sort)` prints the differences.

### The canonical demos, #1592

The demos redone after each milestone and recorded for the TxHDL directory.
The GPU's are the icosahedron series, a board image a step, each step adding one of Razboj's features.
Each image prints `demo ico <n>: <name>` when it starts, then its cycles every 64 frames as the ico demos do, then after 1200 frames, 20 seconds at 60 Hz, `demo ico <n> done: 1200 frames in <m> shown, fps x10 <r>`, and stops with its last frame on the screen.
The rate is the frames drawn against the frames the scanout showed meanwhile, in tenths of a frame a second, and is each demo's measure from one milestone to the next.
It draws `ico <n>: <name>` in the top left corner of both buffers, where the solid never reaches.

| Step | Image | What it shows |
|---|---|---|
| 1 | `demo-ico-1-flat.bin` | Flat faces, back faces culled, Razboj's flat list without GL |
| 2 | `demo-ico-2-depth.bin` | Through GL, depth-tested in tiles, lit per face |
| 3 | `demo-ico-3-smooth.bin` | Lit at each corner along the way from the centre and shaded smooth, so the light runs across the faces as on a ball |
| 4 | `demo-ico-4-textured.bin` | A texture on each face, nearest filtering |
| 5 | `demo-ico-5-mipmapped.bin` | Trilinear mipmaps, over a floor receding to the horizon |
| 6 | `demo-ico-6-fog.bin` | Step 5 in linear fog of the backdrop's colour, the solid drifting from 16 units to 26 and back so that it fades into the fog and out |
| 8 | `demo-ico-8-mirror.bin` | Step 3 over a mirror: the stencil marks the mirror, the solid's reflection is drawn only there, and the mirror is blended over it |

Step 7, blending with smooth alpha, comes once Razboj has smooth alpha (#1520).
A flagship from the same `main`, programmed over JTAG, and the board's serial port open on the board server:

```sh
bazel build //demo/canon:ico_series
bazel run //flagship:flagship_prog -- "${PROG[@]}"
for s in 1-flat 2-depth 3-smooth 4-textured 5-mipmapped 6-fog 8-mirror; do
  bazel run //cpu/vreteno/board/remote:load -- --reset \
      --image=$PWD/bazel-bin/demo/canon/demo-ico-$s.bin --seconds=30 \
      2>&1 | tee board-1592-ico-$s.log
done
```

The images' sums are in `bazel-bin/demo/canon/SHA256SUMS`.
srv records each run from the Cam Link and uploads the video and its log to the TxHDL directory, named as the image is.

The canonical teapot is Newell's Utah teapot, `demo-teapot-1-utah.bin`, the same teapot at every milestone.
Its control points are freeglut's, which `MODULE.bazel` fetches by checksum and `//tools/teapot2rs` writes into `//lib/teapot` at build time.
Its 32 patches are each cut three by three, 564 triangles once the squares that close to a point are one triangle each.
It is smooth shaded, lit from the eye diffusely and specularly, leans 20 degrees towards the eye, and turns about its axis once in 128 frames.
It prints `demo teapot 1: utah`, then `teapot 564 triangles`, and after 256 frames, two turns, `demo teapot 1 done:` with its rate.
Its tessellation, view, light and frames do not change, so that its rate measures the system; a finer teapot would be a demo of its own.
On the machine model with the board's timing, a frame's list takes 48.4 million cycles on one hart, and with both harts two frames take 49.7 million, 4.0 frames a second.
It took 68.2 million and 69.6 million, 2.9 frames a second, before `draw_elements` kept the vertices it had worked out (#1624).
What is left is mostly each triangle's own work, which #1624 says how to take next.

```sh
bazel build //demo/canon:teapot
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=/bazel-bin/demo/canon/demo-teapot-1-utah.bin --seconds=120 \
    2>&1 | tee board-1592-teapot.log
```

Its sum is in `bazel-bin/demo/canon/TEAPOT.SHA256SUMS`.

### Ethernet throughput, #1038

The Ethernet half of #151: the port's throughput through the slots and the DMA engines, against the core's own copy of each frame.
The board has no register path to the MAC, since `EthLite` is not on it, so the baseline is the copy a register path would have to make at least, timed on its own; a path that moved a byte per bus transaction would cost more.
A true register baseline would need `EthLite` on the board as a slot of its own, and a flagship rebuild.

Same bitstream and cable as the scanout above, in the same session.
The flagship has no echo, so the machine at the other end of the cable both counts what the board sends and sends what the board counts, with `ethtest`'s two modes for it.
Its frames have the type `0x88b6`, since the port gives `0x88b5` to the remote peripheral and not to the slots.

```sh
bazel build //cpu/vreteno/rust:ethperf_ram_bin //eth/hil:run
# Once, so that the tester is uploaded and given its capability before
# two copies of it run at the same time.
bazel run //eth/hil:run -- --mode=count --seconds=1
bazel run //eth/hil:run -- --mode=count --seconds=60 \
    2>&1 | tee board-1038-count.log &
bazel run //cpu/vreteno/board/remote:load -- --reset \
    --image=$PWD/bazel-bin/cpu/vreteno/rust/ethperf_ram_bin.bin --seconds=120 \
    2>&1 | tee board-1038.log &
# When board-1038.log says `rx waiting 60`:
for s in 60 60 1514 1514; do
  bazel run //eth/hil:run -- --mode=send --count=1000 --size=$s --seconds=5 \
      2>&1 | tee -a board-1038-send.log
  sleep 3
done
wait
```

The program sends a thousand frames of 60 bytes and a thousand of 1514, each size twice: `tx dma`, the frame written into both transmit slots once and the core only handing the engine a slot and a length; and `tx copy`, the core writing each frame into its slot first, overlapping the previous frame's send.
Then it receives the four bursts in order: `rx dma` notes each frame's number, `rx copy` also copies the frame word by word out of its slot.
The lengths are the frame's bytes without the check sequence, so 60 is the minimum frame on the wire and 1514 the largest.

Each line gives the frames, the timer's ticks from first to last at a hundred million a second, the frames and kilobytes a second, and `cycles/frame`, the core's cycles spent on the frame itself rather than waiting for the port.
A receive line says how many of the thousand arrived, how many were lost, and how many other frames came.
`board-1038-count.log` gives the frames the server counted for each length, and their rate as the server saw it; it should count two thousand of each transmit size.
`board-1038-send.log` gives the rate the server sent at, which bounds what the receive side can show.
Then the whole run once more, with the loader's previous watch over first, since two loaders on the serial port fail, and the bursts paced with `--gap-us=40` for the 60-byte frames and `--gap-us=60` for the 1514: at the line rate the receiver loses most of a burst, and the paced pass shows what it keeps up with.

Pass: `ethperf`, four `tx` lines, four `rx waiting` and `rx` lines, `ethperf done`; and the server counted every frame sent.

Done on October 4, 2026, on the flagship built for #1022: a thousand frames each way through the engines and through the core's own copy.
The results are table `tab:e-perf` in `docs/eth.tex`, and on #1038.
Losses on receive are a result, not a failure: the receiver holds one frame, and a burst faster than the store engine drains it loses frames.
Put the three logs on #151 and #1038, and the numbers into #151's results.

## 8. The DDR3 through MIG, #188

`vreteno_board_pnr` boots the DDR3 test from its boot memory, so it starts as soon as the part is configured; start the watcher first.

```sh
bazel run //cpu/vreteno/board/remote:serial -- --seconds=240 \
    2>&1 | tee board-188-jtag.log &
bazel run //cpu/vreteno:vreteno_board_prog -- "${PROG[@]}"
```

The watcher runs for 240 seconds because programming alone takes about 40 and the test prints after it; a 90-second watcher closed before the first line on September 28.

Done over JTAG on September 28, 2026: the design said `ddr3 ok` (`docs/vreteno.tex`, 46f82d7). The run from the flash below is what is owed.

Pass: `vreteno ddr3 test`, a row of dots, `ddr3 ok`, and dots after it; the third LED, the controller's calibration, lit.
`ddr3 bad` means the memory answered with words it was not given; no `ddr3` at all after the dots means the controller did not calibrate.

Then the same from the flash, since a design in the flash is configured at a different moment from one sent over JTAG:

```sh
bazel run //cpu/vreteno:vreteno_board_flash -- "${PROG[@]}"
```

Power the board off and on, with the watcher running as above into `board-188-flash.log`.
Pass: the same four lines.
Capture: both logs, and a photograph of the LEDs if the serial line says nothing.
It closes #188.

Then put the flagship back: `bazel run //flagship:flagship_flash -- "${PROG[@]}"`, and power the board off and on once more.

## 9. The checks that cannot run yet

### #312, the configuration flash

`STARTUPE2` is in every top, and `flashid` read the chip's identity, `0020ba18`, on the board (`docs/vreteno.tex`).
The layout, programs from `0x00A0_0000`, is in `docs/flagship.tex`, and the flash image carries a program there since d1a2078a.
What #312 still asks of the board is on the issue.

## 10. What was built

Built from `origin/main` at `654ada0`, September 26, 2026.
The sha256 is of the file Bazel wrote; the first sixteen digits are enough to tell two builds apart.

| Target | File | Bytes | sha256 |
|---|---|---|---|
| `//zephyr:fastboot` | `zephyr/fastboot.bin` | 115836 | `65c48a07f6955558` |
| `//cpu/vreteno/rust:trng_ram_bin` | `trng_ram_bin.bin` | 596 | `a5ead28cd89f90aa` |
| `//cpu/vreteno/rust:hello_ram_bin` | `hello_ram_bin.bin` | 148 | `dadce66f7b92d994` |
| `@multitool//tools/fastboot` | `fastboot` | | `bfe2ee0bf34a5d88` |

The four bitstreams, each with the worst setup slack of its routed timing summary:

| Target | Bytes | sha256 | Worst slack |
|---|---|---|---|
| `//cpu/vreteno:vreteno_board_jtag_pnr` | 9730783 | `a22c4aa29d8eedd6` | 0.359 ns |
| `//cpu/vreteno:vreteno_board_boot_pnr` | 9730778 | `8b999cc20b10e679` | 0.213 ns |
| `//cpu/vreteno:vreteno_board_pnr` | 9730778 | `7f0d2e86b7f7d5d4` | 0.213 ns |
| `//flagship:flagship_pnr` | 9730773 | `87b8e249162b78f2` | **-5.725 ns** |

The flagship misses timing, by 5.7 ns over 30174 endpoints: its constraints name the PLL that the switch to the MIG's clock removed, so the crossings between its three clock domains are timed as if they were one (#750).
Until the flagship is rebuilt with a positive slack, section 7 waits and the last step of section 3 is left out: a bitstream that misses timing is not one to write into the flash.
Section 8 then leaves the Vreteno board in the flash rather than the flagship, which is the design that met timing.

All four logs carry one more critical warning, the ring oscillators' false path matching no net (#751); the three Vreteno bitstreams meet timing regardless.

With the clock groups corrected (PR 754), the flagship rebuilt from `e061051` misses by 1.892 ns over 2582 endpoints, all inside the two Ethernet domains: the transmitter reads its frame store straight into the output pins, and the receiver's store is flops that every received byte fans out to (#753).
That was the second thing the flagship waited on.
Both are fixed: rebuilt after #753 (9777b4a9), the flagship meets timing, with a worst slack of +0.341 ns.
