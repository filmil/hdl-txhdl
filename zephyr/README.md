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

The module declares its own roots, so the board `ax7a200b`, the SoC
`vreteno` and the binding for the serial port are found here rather
than in Zephyr's tree.

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
| `soc/hdlfactory/vreteno/` | the SoC: RV32IMC, no atomics, 100 MHz |
| `boards/hdlfactory/ax7a200b/` | the board: the image lives in DDR3 |
| `dts/riscv/hdlfactory/vreteno.dtsi` | the machine, at the addresses it decodes |
| `dts/bindings/serial/` | the binding for the serial port |
| `drivers/serial/uart_vreteno.c` | the console driver, polled |
| `dts/bindings/ethernet/` | the binding for the Ethernet port |
| `drivers/ethernet/eth_vreteno.c` | the Ethernet driver, a port of LiteEth's, over the port's frame engines |
| `dts/bindings/rng/` | the binding for the entropy source |
| `drivers/entropy/entropy_vreteno.c` | the entropy driver, which the network stack's random numbers come from |
| `fastboot/` | fastboot over TCP: the protocol, its host harness and tests, and the server for the board |

## Fastboot

A program reaches the board over the network with stock `fastboot`,
from Android's platform tools, and nothing written for the host
(issue 143):

```sh
fastboot -s tcp:192.168.1.50 boot program.bin
```

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
`boot` copies the program to `0x4000_0000`, the address the serial
loader uses too, and jumps.
The copy overwrites the Zephyr that is doing it, so it runs from a few
words of position-independent assembly moved first to the staging
area's last page, `fastboot/app/src/jump.S`.
A program longer than 16 MiB is refused, because the Ethernet engines
store received frames at `0x4100_0000` and go on doing so after the
jump.

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

## Why a driver rather than a 16550

The serial port is three registers: a byte written to the first goes
out, the second is the status, and a read of the third takes the
oldest byte received.
Growing it a 16550 register map would mean a line control register, a
line status register, an interrupt enable and a divisor latch inside a
peripheral the bootloader and every board test depend on, to save
about eighty lines of driver.
The driver is the reversible choice and it touches no hardware that is
already proven.

If Linux is taken up, issue 279, that calculus changes: Linux has an
8250 driver too, and a 16550 map would then pay twice.
The decision is recorded here so that the second person to ask does
not have to work it out again.

## What checks it

`//cpu/vreteno:zephyr_port_test`.

Zephyr's own build cannot check the port against the design, because
Zephyr is not in this tree and the design is not in Zephyr's.
So the addresses here are held to the constants the hardware itself
uses: the serial port at `UART_BASE`, the machine timer's `mtime` and
`mtimecmp` at the offsets above `CLINT_BASE` that Zephyr's own driver
reads, the interrupt controller where RISC-V machines put it, and the
DDR3 where the router decodes it.
The driver's register map and the SoC's instruction set are checked
the same way, and so are the two things that have already made this
port build an image with no console: the driver selecting
`SERIAL_HAS_DRIVER`, without which `UART_CONSOLE` is not even a
visible symbol, and the driver reaching the architecture's
`sys_read32` rather than leaving it implicit.

Moving the serial port by one page in the device tree fails that test,
which was tried rather than assumed.

## What is not done

The image has not run.
`west build` producing an ELF needs no board; the image printing on
the serial port, and `samples/synchronization` exercising the
scheduler, the timer interrupt and context switching, both do.
Those are issue 277's second and third acceptance bullets and they
wait for hardware.

What an ELF does prove, as of this change, is that the module is
reached: `CONFIG_UART_VRETENO` and `CONFIG_UART_CONSOLE` are set in
the generated configuration, `uart_vreteno_poll_out` is in the image,
and it compiles to a spin on bit 0 of the status register followed by
a store of the byte, which is what the hardware asks for.
An image of 14612 bytes links at `0x4000_0000`, RV32 with compressed
instructions and the soft-float ABI.

## Building it hermetically

Bazel builds images from this module with nothing from the system
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
