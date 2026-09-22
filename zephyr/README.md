<!-- SPDX-License-Identifier: Apache-2.0 -->
# Zephyr on Vreteno

This directory is a Zephyr module: a SoC, a board, a device tree and a
console driver that describe the Vreteno machine to Zephyr.
Zephyr itself is not vendored here and is not part of the Bazel build.

## Building an image

Point any Zephyr at this directory.

```sh
west build -b ax7a200b samples/hello_world \
  -- -DZEPHYR_EXTRA_MODULES=/path/to/txhdl/zephyr
```

The module declares its own roots, so the board `ax7a200b`, the SoC
`vreteno` and the binding for the serial port are found here rather
than in Zephyr's tree.

## What is here

| Path | What |
|---|---|
| `zephyr/module.yml` | the manifest, and the roots |
| `soc/hdlfactory/vreteno/` | the SoC: RV32IMC, no atomics, 100 MHz |
| `boards/hdlfactory/ax7a200b/` | the board: the image lives in DDR3 |
| `dts/riscv/hdlfactory/vreteno.dtsi` | the machine, at the addresses it decodes |
| `dts/bindings/serial/` | the binding for the serial port |
| `drivers/serial/uart_vreteno.c` | the console driver, polled |

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
uses: the serial port at `UART_BASE`, the timer at `CLINT_BASE` with
`msip`, `mtimecmp` and `mtime` at the offsets a stock CLINT driver
expects, the interrupt controller where RISC-V machines put it, and
the DDR3 where the router decodes it.
The driver's register map and the SoC's instruction set are checked
the same way.

Moving the serial port by one page in the device tree fails that test,
which was tried rather than assumed.

## What is not done

The image has not run.
`west build` producing an ELF needs no board; the image printing on
the serial port, and `samples/synchronization` exercising the
scheduler, the timer interrupt and context switching, both do.
Those are issue 277's second and third acceptance bullets and they
wait for hardware.

Building Zephyr from this repository's own hermetic Bazel, rather than
from a Zephyr the reader supplies, is a separate piece of work: it
wants CMake, ninja and a Python with the devicetree tooling, none of
which this build has.
