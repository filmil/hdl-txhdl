/* SPDX-License-Identifier: Apache-2.0 */
OUTPUT_ARCH("riscv")
ENTRY(_start)
SECTIONS
{
  . = 0x80000000;
  .text : { *(.text.init) *(.text*) }
  .rodata : { *(.rodata*) *(.srodata*) }
  .tohost : { *(.tohost) }
  .data : { *(.data*) *(.sdata*) }
  .bss : { *(.bss*) *(.sbss*) *(COMMON) }
}
