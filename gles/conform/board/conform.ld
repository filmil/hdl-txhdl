/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The conformance suite's memory on the board (#999), as Doom's (issue
 * 1177): the program, its data and its heap in the DDR3 from 0x4000_0000
 * up to Razboj's framebuffer at 0x4200_0000, 32 MiB; the stack the core's
 * own data memory, 64 KiB at 0x0001_0000.
 */
ENTRY(_start)

MEMORY
{
  ram (rwx) : ORIGIN = 0x40000000, LENGTH = 32M
  stack (rw) : ORIGIN = 0x00010000, LENGTH = 64K
}

SECTIONS
{
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

  . = ALIGN(16);
  __heap_start = .;
  __heap_end = ORIGIN(ram) + LENGTH(ram);
  __stack_top = ORIGIN(stack) + LENGTH(stack);
}
