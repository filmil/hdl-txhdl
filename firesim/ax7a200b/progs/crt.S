// SPDX-License-Identifier: Apache-2.0
// Start-up for one hart: a stack, then main; its return value ends the
// simulation through tohost (1 | code << 1, as riscv-tests do).
    .section .text.init, "ax", @progbits
    .globl _start
_start:
    csrr t0, mhartid
    bnez t0, park
    la sp, stack_top
    call main
    slli a0, a0, 1
    ori a0, a0, 1
    la t0, tohost
    sd a0, 0(t0)
park:
    wfi
    j park
    .section .tohost, "aw", @progbits
    .balign 64
    .globl tohost
tohost:
    .dword 0
    .balign 64
    .globl fromhost
fromhost:
    .dword 0
    .bss
    .balign 16
    .space 16384
stack_top:
