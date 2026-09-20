/* SPDX-License-Identifier: Apache-2.0 */
/*
 * Vreteno's memory as the linker sees it.
 *
 * Two regions. The instruction memory holds the program and its
 * constants: the core fetches from its own copy inside itself, and the
 * same words sit on the bus at address zero, read-only, so a load
 * reaches `.rodata` there. Everything a program writes lives in the
 * data memory, and the image carries its initial bytes.
 *
 * Both are 4 KiB and both are hard limits: the core fetches by
 * `pc[11:2]`, so a program past 4096 bytes wraps into itself
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
  } > imem

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
