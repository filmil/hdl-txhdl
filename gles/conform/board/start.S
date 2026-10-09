/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The conformance suite's entry on the board (#999), as Doom's (issue
 * 1177): where the serial loader and fastboot's boot jump, the start of
 * the image at 0x4000_0000. The stack is the core's own data memory, as
 * the Rust programs' is (issue 1278); the uninitialised data is zeroed;
 * a trap goes to `trap`; then `main`, and if it returns, the core stops
 * by writing `mhalt`.
 */
	.section .text.init
	.globl _start
_start:
	la	sp, __stack_top
	la	t0, trap
	csrw	mtvec, t0
	la	t0, __bss_start
	la	t1, __bss_end
1:	bgeu	t0, t1, 2f
	sw	zero, 0(t0)
	addi	t0, t0, 4
	j	1b
2:	call	main
	csrwi	0x7c0, 1
3:	j	3b

/* A trap (#1536): the cause, the instruction and the value it names are
 * said on the serial port by `conform_trap`, and the core stops, rather
 * than running on at an address nothing set and saying nothing. */
	.align	2
trap:
	csrr	a0, mcause
	csrr	a1, mepc
	csrr	a2, mtval
	call	conform_trap
	csrwi	0x7c0, 1
4:	j	4b
