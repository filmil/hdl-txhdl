/* SPDX-License-Identifier: Apache-2.0 */
/*
 * Vreteno's memory as the linker sees it.
 *
 * Two regions, and which section goes where is forced by the machine
 * rather than by convention. Instruction memory holds the program and
 * nothing else: it is a memory inside the core, not a device on the
 * bus, so a load from it never leaves the core and reads nothing. A
 * constant placed there would be a constant the program cannot read.
 * Everything a load may touch, `.rodata` included, therefore lives in
 * the data memory, and the image carries its initial bytes.
 *
 * Both are 4 KiB and both are hard limits: the core fetches
 * `pc[11:2]`, so a program past 1024 instructions wraps into itself
 * with no fault, and the data memory's word index is ten bits.
 * `elf2vreteno` refuses an image that overruns either.
 */
ENTRY(_start)

MEMORY
{
  imem (rx) : ORIGIN = 0x00000000, LENGTH = 4K
  dmem (rw) : ORIGIN = 0x00001000, LENGTH = 4K
}

SECTIONS
{
  /* The reset vector is address zero, so the entry stub goes first. */
  .text : {
    *(.text.init)
    *(.text .text.*)
  } > imem

  .rodata : ALIGN(4) {
    *(.rodata .rodata.*)
    *(.srodata .srodata.*)
  } > dmem

  .data : ALIGN(4) {
    *(.data .data.*)
    *(.sdata .sdata.*)
  } > dmem

  .bss (NOLOAD) : ALIGN(4) {
    __bss_start = .;
    *(.bss .bss.*)
    *(.sbss .sbss.*)
    *(COMMON)
    . = ALIGN(4);
    __bss_end = .;
  } > dmem

  /* The stack grows down from the top of the data memory. Nothing
     sets the stack pointer on this machine, so the entry stub does. */
  __stack_top = ORIGIN(dmem) + LENGTH(dmem);

  /DISCARD/ : {
    *(.eh_frame)
    *(.eh_frame_hdr)
    *(.comment)
    *(.riscv.attributes)
  }
}
