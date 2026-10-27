// SPDX-License-Identifier: Apache-2.0
//
// The smallest program for FireSim's Rocket design (issue 1988): print a
// line on the UART and stop. The driver loads it into the target's DRAM
// over TSI; the UART bridge prints what the target sends; writing 1 to
// `tohost` ends the simulation with exit code 0, as riscv-tests do.
//
// The UART is Chipyard's SiFive UART at its default 0x10020000: txdata
// at 0x00 (bit 31 set while the FIFO is full) and txctrl at 0x08 (bit 0
// enables the transmitter). Its divisor is FireSim's from reset.
    .section .text.init, "ax", @progbits
    .globl _start
_start:
    csrr t0, mhartid
    bnez t0, park
    li t0, 0x10020000
    li t1, 1
    sw t1, 8(t0)
    la a0, msg
next:
    lbu t1, 0(a0)
    beqz t1, done
full:
    lw t2, 0(t0)
    bltz t2, full
    sw t1, 0(t0)
    addi a0, a0, 1
    j next
done:
    la t0, tohost
    li t1, 1
    sd t1, 0(t0)
park:
    wfi
    j park

    .section .rodata
msg:
    .string "Hello from Rocket on the AX7A200B, over Ethernet.\n"

    // fesvr finds these two by their symbols.
    .section .tohost, "aw", @progbits
    .balign 64
    .globl tohost
tohost:
    .dword 0
    .balign 64
    .globl fromhost
fromhost:
    .dword 0
