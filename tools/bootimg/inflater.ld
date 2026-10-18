/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The boot image's inflater (issue 1229), linked where the packer puts
 * it: past the shim, at `bootimg`'s INFLATER, and below OpenSBI's load
 * address, which no part of the image is moved under, so the moves that
 * run before it never write over it.
 *
 * It has text and constants only. Nothing sets up data or zeroes a BSS
 * before the shim calls it, so the link fails if either appears.
 */
ENTRY(_start)

MEMORY
{
  ram (rx) : ORIGIN = 0x40001000, LENGTH = 0x7f000
}

SECTIONS
{
  /* The shim calls the image's first byte, so the entry goes first. */
  .text : {
    *(.text.init)
    *(.text .text.*)
  } > ram

  .rodata : ALIGN(4) {
    *(.rodata .rodata.*)
    *(.srodata .srodata.*)
  } > ram

  .data : {
    *(.data .data.*)
    *(.sdata .sdata.*)
    *(.bss .bss.*)
    *(.sbss .sbss.*)
    *(COMMON)
  } > ram

  ASSERT(SIZEOF(.data) == 0, "the inflater may have no data or BSS")

  /DISCARD/ : {
    *(.eh_frame)
    *(.eh_frame_hdr)
    *(.comment)
    *(.riscv.attributes)
  }
}
