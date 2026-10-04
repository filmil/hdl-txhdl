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
    //cpu/vreteno/rust:ddr3bw_ram_bin
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

Pass: `tap/device found: 0x13636093`, `Examined RISC-V core; found 1 harts` with `XLEN=32` and `misa=0x40001104`, `state halted`, a `pc`, the four words, a `stepped pc` one instruction on, and `state running`, which is the session `//cpu/vreteno:openocd_test` runs against the simulated board.
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

Pass: the loader answers `ok` and the address it loaded to, `40000000`, and then the program prints `hello from rust`; `ok` comes only when the loader's sum matches, which is otherwise `bad sum`.
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
