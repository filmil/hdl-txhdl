<!-- SPDX-License-Identifier: Apache-2.0 -->
# Zephyr on Vreteno

This directory is a Zephyr module: a SoC, a board, a device tree and
the drivers that describe the Vreteno machine to Zephyr.
Zephyr itself is not vendored here.
Bazel fetches Zephyr 4.1.0 by checksum and builds images against this
module with this repository's own toolchain; see "Building it
hermetically" below.

## Building an image

Point any Zephyr at this directory.

```sh
west build -b ax7a200b samples/hello_world \
  -- -DZEPHYR_EXTRA_MODULES=/path/to/txhdl/zephyr
```

The module declares its own roots, so the board `ax7a200b` and the SoC
`vreteno` are found here rather than in Zephyr's tree.

Driving CMake directly, without `west`, needs a different flag.
`ZEPHYR_EXTRA_MODULES` is collected and then passed to a script that
Zephyr runs only `if(WEST OR ZEPHYR_MODULES)`, so without west it is
read and discarded.
The build then succeeds and the module is simply absent: no driver, no
console, and an ELF that says nothing on a board.
Name the module instead.

```sh
cmake -B build -S samples/hello_world -GNinja \
  -DBOARD=ax7a200b \
  -DZEPHYR_MODULES=/path/to/txhdl/zephyr
```

## What is here

| Path | What |
|---|---|
| `zephyr/module.yml` | the manifest, and the roots |
| `soc/hdlfactory/` | the family, which is what the hardware model reads |
| `soc/hdlfactory/vreteno/` | the SoC: RV32IMAC, 100 MHz |
| `boards/hdlfactory/ax7a200b/` | the board: the image lives in DDR3 |
| `dts/riscv/hdlfactory/vreteno.dtsi` | the machine, at the addresses it decodes |
| `dts/bindings/ethernet/` | the binding for the Ethernet port |
| `drivers/ethernet/eth_vreteno.c` | the Ethernet driver, a port of LiteEth's, over the port's frame engines |
| `dts/bindings/rng/` | the binding for the entropy source |
| `drivers/entropy/entropy_vreteno.c` | the entropy driver, which the network stack's random numbers come from |
| `drivers/entropy/vreteno_condition.c` | the driver's conditioner: SHA-256 from Mbed TLS over 32 of the source's words for each 32 bytes (issue 780) |
| `tests/` | the conditioner's host tests: known answers, and a source shaped like the board's through it, judged by `//tools/trngstat` |
| `include/vreteno/regs/` | the register headers the drivers include, written from the parts' `regmap!` maps (issue 709) |
| `fastboot/` | fastboot over TCP: the protocol, its host harness and tests, and the server for the board |
| `entropy/app/` | the entropy driver's conditioned output on the serial port, in the format `//tools/trngstat` reads: the board's check of the conditioner |

## Fastboot

A program reaches the board over the network with stock `fastboot`,
from Android's platform tools, and nothing written for the host
(issue 143):

```sh
fastboot -s tcp:192.168.1.50 boot program.bin
```

A program shorter than 1580 bytes is refused by the tool as
`too short` before anything is sent: it reads a boot image header's
worth of a file to decide whether the file is already one (issue 799).
Pad a small program with zeros; the padding lies past its end and is
never read. `vreteno_fastboot` in `//cpu/vreteno/rust:defs.bzl` pads a
flat program to 4096 bytes, as `hello_fastboot` does for
`hello_ram_bin`, and by hand it is `truncate -s '>4096' program.bin`.

`//zephyr:fastboot` is the image that listens for it: a TCP server on
port 5554, which is where AOSP's `fastboot/README.md` puts the device.
The address is static by default, `CONFIG_NET_CONFIG_MY_IPV4_ADDR` in
`fastboot/app/prj.conf`, since fastboot's device is the server and the
host has to know where it is.
A network with a DHCP server wants `NET_DHCPV4` instead.
Load the image once with the serial loader, and from then on a program
goes over the wire.

`fastboot boot` wraps a file that is not an Android boot image in one,
and the server takes the program back out of it.
The download is staged in DDR3 at `0x4800_0000`, 16 MiB of it reserved
in `fastboot/app/boards/ax7a200b.overlay`.
It is copied there a word at a time whatever the alignment fastboot's
framing leaves, by `fb_copy` in the core, where `memcpy` copied a byte
at a time: 41 cycles a byte against 11 in `board_test`'s model of the
DDR3, and 265 a byte on the board before (issue 1230).
`boot` copies the program to `0x4000_0000`, the address the serial
loader uses too, and jumps.
The copy overwrites the Zephyr that is doing it, so it runs from a few
words of position-independent assembly moved first to the staging
area's last page, `fastboot/app/src/jump.S`.
A download over `0x00fff000` bytes, the staging area less the last
page the copy routine takes, is refused, so a program is always
shorter than that; separately, `boot` refuses a program over 16 MiB,
because the Ethernet engines store received frames at `0x4100_0000`
and go on doing so after the jump.
The host test's server has no jump page and offers the full 16 MiB.

`//zephyr:fastboot_profile` is the same server saying where a
download's time goes (issue 1230), with `fastboot/profile.conf`.
When a connection closes it prints the bytes and the cycles it took, at
the rate the line gives.
It prints the cycles spent in `recv` and in the fastboot core, and of
those in the copy to the staging area.
It prints the Ethernet driver's frames and cycles each way, the copy
into or out of a slot apart, and how often a send waited for the
transmitter (`CONFIG_ETH_VRETENO_PROFILE`).
It prints every thread's cycles across the connection, from Zephyr's
runtime statistics.
The driver's counts are `eth_vreteno_prof` in
`include/vreteno/eth_vreteno.h`.

`getvar`, `download` and `boot` are answered.
`flash` and `erase` fail, since nothing on this machine writes
persistent storage, and so does `reboot`, since the SoC cannot reset
itself.

What checks it, off the board:

* `//zephyr:fastboot_test`: stock `fastboot` 35.0.2, pinned in
  `multitool.lock.json`, reads `max-download-size` and boots a 100 003
  byte image through `//zephyr:fastboot_host`, which runs the board's
  protocol code on a host socket.
  The program it stages must be the image, byte for byte.
* `//zephyr:fastboot_core_test`: the protocol code fed one session in
  pieces from one byte to all of it, because a wire splits a message
  where loopback does not, and the refusals.
* `//cpu/vreteno:fastboot_jump_test`: the copy-and-jump, assembled from
  `jump.S`, run on the core's model from places it was not assembled
  for.
* `//zephyr:fastboot`: the server builds against this port with a
  network stack.

What needs the board: a program received over the real port and run.

## Why the serial port is SiFive's

The serial port's registers are SiFive's `sifive,uart0` (issue 1011), and the
console is Zephyr's own `uart_sifive` driver, which the device tree's node
binds and the board's `CONFIG_UART_SIFIVE_PORT_0=y` turns on.
Until then the port was three registers of its own, with a driver of
its own here, and this section argued that a driver was cheaper than
growing the hardware a standard map.
Linux on this core, issue 279, changed that: Linux, OpenSBI and Zephyr
all drive SiFive's map with drivers somebody else maintains, so the
hardware took the map once rather than this repository carrying a
driver for each.
SiFive's map is the smaller of the standard ones, seven registers, and
the port kept its reset behaviour by starting with both enables and the
receive interrupt on.

## What checks it

`//cpu/vreteno:zephyr_port_test`.

Zephyr's own build cannot check the port against the design, because
Zephyr is not in this tree and the design is not in Zephyr's.
So the addresses here are held to the constants the hardware itself
uses: the serial port at `UART_BASE`, the machine timer's `mtime` and
`mtimecmp` at the offsets above `CLINT_BASE` that Zephyr's own driver
reads, the interrupt controller where RISC-V machines put it, and the
DDR3 where the router decodes it.
The serial port is SiFive's `sifive,uart0` (issue 1011), driven by
Zephyr's stock `uart_sifive`, so the test holds every offset and bit
that driver hard-codes to the hardware's own map, and checks that the
device tree's console is a `sifive,uart0` node with its clock.
The SoC's instruction set is checked the same way, and so are the
things that would build an image with no console: the board turning
the driver's first port on, the SoC naming the peripheral clock the
driver divides, `soc.h`'s `SIFIVE_PERIPHERAL_CLOCK_FREQUENCY`, and
the board enabling `UART_CONSOLE`.

Moving the serial port by one page in the device tree fails that test,
which was tried rather than assumed.

The drivers name their registers through headers written from the
hardware's own declarations: every peripheral in the parts declares its
registers with `regmap!`, `//tools/regmap` writes a C header from each
map, and the drivers include `<vreteno/regs/<map>.h>` rather than
naming an offset (issue 709).
The headers are committed, since a reader's `west build` of this module
runs no Bazel.
`REG_MAPS` in `BUILD.bazel` names every map, and `write_source_files`
keeps each committed header equal to what its map writes (issue 1179):
a header that differs fails `//zephyr:regs_update`'s test for it,
and `bazel run //zephyr:regs_update` writes them all again after a map
changes.
`//zephyr:regs_maps_test` holds `REG_MAPS` to the maps `//tools/regmap`
lists and to the headers committed, so a map added to the parts and not
to the list, or a header missing, fails and names the fix.

## What has run

Zephyr has run on the board, on 2026-09-22 (issue 277).
The images were built by hand from the branch of PR #410, not by
Bazel, and were sent by the serial loader into `vreteno_board_boot_pnr`.
`samples/hello_world` printed its banner and
`Hello World! ax7a200b/vreteno` on its first execution, which proves
the console driver, the load and the reset vector on hardware.
`samples/synchronization` then ran for two minutes: 207
`Hello World from` lines, `thread_a` and `thread_b` strictly
alternating, against 208.3 predicted from the sample's 600 ms period.
So the machine timer, its interrupt, the scheduler and context
switching work, and `mtime` crossed its 32-bit boundary with the
cadence unchanged.

The transcripts are in `runs/2026-09-22/`, as the serial line gave
them, with only Bazel's own lines taken out.

| File | What ran |
|---|---|
| `first-run-serial.log` | `hello_world`, its first execution |
| `sync-run-serial.log` | `synchronization`, after a serial reset, 42 lines watched |
| `sync-run-2min-serial.log` | `synchronization` from a freshly programmed part, 125 s |
| `sync-over-sync-reset-125s.log` | `synchronization` reset-loaded over a running copy of itself |
| `hello-over-sync-reset-125s.log` | `hello_world` reset-loaded over a running `synchronization` |

One reset-load of `synchronization` over a running copy, on the same
day, printed nothing after `boot`; it did not reproduce, and issue
419, a reset that left the control registers as Zephyr set them, was
found and fixed behind it.

The images themselves are not in the tree.
The one that printed was 14612 bytes; `//zephyr:hello_world` is a
later build, 14644 bytes today, with the drivers changed since, the
register headers of issue 709 among them, and it has not itself been
run on the board.

What an ELF proves without the board is that the module is reached:
`CONFIG_UART_SIFIVE`, `CONFIG_UART_SIFIVE_PORT_0` and
`CONFIG_UART_CONSOLE` are set in the generated configuration, beside
the SoC and the board.
An image links at `0x4000_0000`, RV32 with compressed instructions and
the soft-float ABI.

On 2026-09-28 Zephyr ran on the flagship as fastboot (issue 143), sent
by the serial loader into flagship `9755f9c85be5d066`, built with PR 795
and PR 791, issue 786's two fixes: the SoC selects `RISCV_HAS_PLIC`,
and the device tree's `riscv,ndev` counts source 0.
It listened on port 5554, answered `getvar max-download-size` with
`0x00fff000`, and booted a program sent with `fastboot boot`, which
printed `hello from rust`.
The same three checks passed again on 2026-10-01, on main `b747d42`'s
flagship; both runs are recorded on issue 143.

## Building it hermetically

Bazel builds images from this module with every tool it names fetched
by checksum, and only the shell's own commands from the system
(issue 390): `zephyr_image` in `zephyr/defs.bzl` hands Zephyr's own
CMake, Kconfig and Python every tool by checksum and takes the ELF,
the raw image and the generated configuration out.

| Target | What it is | What checks it |
|---|---|---|
| `//zephyr:hello_world` | Zephyr's `samples/hello_world` for `ax7a200b` | `//zephyr:config_test` |
| `//zephyr:hello_world_net` | the same with `net.conf`, a network stack | `//zephyr:eth_config_test` |
| `//zephyr:fastboot` | the fastboot server, an application in `fastboot/app` | the fastboot tests above |

The two configuration tests read the `.config` each build generated,
because an ELF existing proves very little, as this port showed four
separate times: each built an ELF that would not have printed.
`config_test` wants the serial driver, the console, the SoC, the board
and the machine timer; `eth_config_test` wants the Ethernet driver,
Ethernet L2 and the entropy driver.
`//zephyr:paths_test` holds the image to naming no path of the machine
that built it, neither the sandbox nor the execution root, so that two
builds of one tree give one `.elf` and one `.config` (issue 711).
All of them are in `bazel build //...` and `bazel test //...`.

The versions it is built with are in `MODULE.bazel`; these are the
ones that matter to Zephyr, with why where it is not obvious.

| Module | Version | Note |
|---|---|---|
| `rules_python` | 2.3.4 | the interpreter, and the pip packages |
| `rules_foreign_cc` | 0.16.0 | |
| `rules_dtc` | 0.0.5 | |
| `dtc` | 1.7.2.bcr.1 | |
| `flex` | 2.6.4.bcr.6 | the pin is required; see below |
| `ninja` | 1.13.2 | |
| CMake | 3.31.6 | an `http_archive`, sha256 `5a1133ff103c71eb5120e2cc3de922733e7d8a26a98ae716397e8676adb367bf` |
| `riscv_none_elf_gcc` | 14.2.0 | |

Three things cost time getting there, and would cost it again.

**`flex` must be pinned to `2.6.4.bcr.6`.**
The default resolution takes `2.6.4.bcr.2`, on which building `dtc`
fails with `config.h:208: expected expression before '/'` and a
`strrchr` called with too few arguments.

**The pip packages need their transitive closure listed.**
`rules_python` installs what is named and nothing else, so the first
run died on `No module named 'six'`.

**The rule passes `-DZEPHYR_MODULES`, not `-DZEPHYR_EXTRA_MODULES`.**
See "Building an image" at the top of this file: without `west` the
latter is collected and discarded, and the build would fetch Zephyr,
run CMake, produce an ELF, pass, and be testing a Zephyr with none of
this repository in it.
