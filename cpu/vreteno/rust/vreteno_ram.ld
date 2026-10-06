/* SPDX-License-Identifier: Apache-2.0 */
/*
 * Vreteno's memory as the linker sees it for a program that is loaded
 * rather than built into the netlist.
 *
 * The core fetches from the bus above the boot memory, so a program
 * sent down the serial port lives in the board's DDR3 and runs there.
 * One region is enough: this is memory on the bus, read and written,
 * so the text, the constants and the data sit together and a program
 * can read its own constants. The boot memory is on the bus too since
 * issue 268, but read-only, which is why vreteno.ld splits a program
 * built into it.
 *
 * The region starts where the router puts the memory and is as much of
 * it as a loaded program is allowed: a megabyte, which is far more
 * than anything this machine runs today and far less than the board
 * holds, so a runaway length is refused rather than scribbled over the
 * rest of the memory.
 */
ENTRY(_start)

MEMORY
{
  ram (rwx) : ORIGIN = 0x40000000, LENGTH = 1M
}

SECTIONS
{
  /* The loader jumps to the address it was given, which is the start
     of the region, so the entry stub goes first. */
  .text : {
    *(.text.init)
    *(.text .text.*)
  } > ram

  .rodata : ALIGN(4) {
    *(.rodata .rodata.*)
    *(.srodata .srodata.*)
  } > ram

  .data : ALIGN(4) {
    *(.data .data.*)
    *(.sdata .sdata.*)
  } > ram

  .bss (NOLOAD) : ALIGN(4) {
    __bss_start = .;
    *(.bss .bss.*)
    *(.sbss .sbss.*)
    *(COMMON)
    . = ALIGN(4);
    __bss_end = .;
  } > ram

  /* The stack grows down from the top of the stack memory, 64 KiB of
     block RAM at 0x10000, the HAL's map::STACK, rather than from the
     top of this region: the DDR3 is not cached for data, so a stack
     here paid the controller's latency on every spill and reload,
     39% of a frame of ico_gl's cycles (issue 1278). */
  __stack_top = 0x00010000 + 64K;

  /DISCARD/ : {
    *(.eh_frame)
    *(.eh_frame_hdr)
    *(.comment)
    *(.riscv.attributes)
  }
}
