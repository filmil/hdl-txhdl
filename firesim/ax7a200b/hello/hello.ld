/* SPDX-License-Identifier: Apache-2.0 */
/* The target's DRAM starts at 0x80000000, where Rocket's boot ROM
   jumps once the driver has loaded the program. */
OUTPUT_ARCH("riscv")
ENTRY(_start)
SECTIONS
{
  . = 0x80000000;
  .text : { *(.text.init) *(.text*) }
  .rodata : { *(.rodata*) }
  .tohost : { *(.tohost) }
  .data : { *(.data*) }
  .bss : { *(.bss*) }
}
