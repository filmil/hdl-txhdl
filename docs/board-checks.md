<!-- SPDX-License-Identifier: Apache-2.0 -->
# The board checks that are owed

Status: written September 26, 2026, while the board server was unreachable.
Author: automated coding assistant, with human supervision.

Seven issues wait on the AX7A200B, and each of them says what it wants from the board in its own comments.
This file puts those wants in one place, as commands to run in order, with what each should print and what to keep.
Four of the checks can run on the next board session; the fifth, fastboot, waits on a flagship that meets timing (#750 and #753); two cannot run yet, and section 9 says what each still lacks.

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
    //cpu/vreteno/rust:trng_ram_bin //cpu/vreteno/rust:steps_ram_bin
bazel build //zephyr:fastboot @multitool//tools/fastboot
bazel build //tools/trngstat
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

Steps 3 and 5 wait on #750 and #753, as section 10 says.

## 4. The debug module, #154

Towards #154: this is step 2's board proof; the gdb and OpenOCD transcript is step 3's, and section 9 says why it cannot run yet.

```sh
bazel run //cpu/vreteno:vreteno_board_jtag_prog -- "${PROG[@]}"
bazel run //cpu/vreteno:vreteno_board_dm_probe 2>&1 | tee board-154-dm.log
```

Pass: every line that begins `dm:` ends in `ok` or gives a value, none says `BAD`, and the last is `dm: halt, registers, resume: every step ok`.
Capture: `board-154-dm.log`, with `grep '^dm:'` of it pasted into the issue.
It closes nothing; #154 stays open for step 3.

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

## 7. Fastboot, #143

This check waits on #750 and #753: the flagship bitstream built on September 26 misses timing, and section 10 says why.

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

`hello_fastboot` is `hello_ram_bin` padded with zeros to 4096 bytes.
Stock `fastboot` refuses the 148 bytes of `hello_ram_bin` as `too short`, before it sends anything, because it reads a boot image header's worth of a file first (#799); `//zephyr:fastboot_test` checks both off the board.

The fastboot server is 28959 words, and at one acknowledgement a word it takes about 90 seconds to send.
`load` counts `--seconds` from the end of the transfer and does not cut a transfer that is still moving, so the 60 are for watching alone; fastboot said `listening` about 15 seconds after its transfer ended (#784).
`load` says how far it has got every 4096 words, and a transfer that stops says at which word and fails.
The serial watcher and the loader both hold the serial port, so the watcher in step 3 starts after the loader's `--seconds` have run out, or the loader's own output, which runs that long, is read for `hello from rust` instead.
If the ping to `192.168.1.50` from the board server fails, the link is the first thing to look at: `ip link show fpga-a200t-eth0` should say `LOWER_UP`.
Capture: the three logs.
It closes #143 when all three pass.

## 8. The DDR3 through MIG, #188

`vreteno_board_pnr` boots the DDR3 test from its boot memory, so it starts as soon as the part is configured; start the watcher first.

```sh
bazel run //cpu/vreteno/board/remote:serial -- --seconds=240 \
    2>&1 | tee board-188-jtag.log &
bazel run //cpu/vreteno:vreteno_board_prog -- "${PROG[@]}"
```

The watcher runs for 240 seconds because programming alone takes about 40 and the test prints after it; a 90-second watcher closed before the first line on September 28.

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

Then, once the flagship meets timing, put the flagship back: `bazel run //flagship:flagship_flash -- "${PROG[@]}"`, and power the board off and on once more.

## 9. The checks that cannot run yet

### #151, direct memory access for Ethernet and HDMI

The issue's done-when asks for throughput against the register path, for Ethernet and for HDMI, measured on the board.
None of the three things that needs exists yet.

* **A measurement program.** Nothing in `cpu/vreteno/rust/` times a transfer: one that sends and receives a fixed number of frames through the slots and through the registers, and prints the time each took from the timer, is the Ethernet half.
* **The scanout on the board.** `ex_scanout` and `ex_dmawb` prove the pieces off-board, and nothing joins them to the video peripheral in a bitstream, so HDMI has no DMA path on the board to measure.
* **A baseline.** The register path has never been timed on the board either, so the comparison needs both runs.

#151 stays open.

### #312, the configuration flash

Nothing board-side exists: `STARTUPE2` is not instantiated in any top, no program reads the flash's JEDEC identity, and the layout of programs beside the bitstream is not written down.
#312 lists the four pieces; each is work before any board time.

### #154, the gdb and OpenOCD transcript

Step 3 of #154, the Debug Transport Module on `BSCANE2` that OpenOCD reaches, is not in the tree.
Until it is, OpenOCD has nothing to talk to, and the probe in section 4 is the whole of what the board can show.

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
That is the second thing the flagship waits on.
