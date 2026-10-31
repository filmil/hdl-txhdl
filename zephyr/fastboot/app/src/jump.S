/* SPDX-License-Identifier: Apache-2.0 */
/*
 * Copy a program over the image that is running, and jump to it
 * (issue 143).
 *
 * `boot` puts the program at 0x4000_0000, which is where this Zephyr
 * is running, so the copy cannot run from here: it would overwrite
 * itself half way. These instructions are copied first to the last
 * page of the staging area, which a download never reaches, and run
 * from there. They name no address of their own, every branch is
 * relative and every pointer comes in a register, so they run wherever
 * they are put.
 *
 *     a0  the program's first word in the staging area
 *     a1  where it goes, and where to jump
 *     a2  its length in bytes, rounded up to a word
 *     a3  the serial port's transmit register, or zero for silence
 *
 * Interrupts are off before this is reached, so nothing runs between
 * the copy and the jump.
 *
 * With a serial port, the routine reports on its way (issue 2011):
 * once the copy is done, `jump` and the sum of the program's words as
 * the destination holds them, then `go` once the instruction cache is
 * empty, just before the jump. `boot` prints the same sum over the
 * staging area on its `booting` line, outside the time from that line
 * to the program's first, which the nightly measures. A board that
 * stops after `fastboot: booting` then says where: no `jump` line,
 * the copy; a sum that differs from `booting`'s, a word that landed
 * wrong; `go` and nothing after it, the program itself or its fetch. Zephyr's console
 * is gone by then, so the bytes go to the register directly, each
 * once the port says it has room (bit 31 of a read, as `sifive,uart0`
 * has it).
 *
 *     a4  where the program went, kept for the jump
 *     a6  its length, kept for the sum
 *     a7  the return address of the two small routines below
 *
 * Before the jump `a3` is set back to where the program went, as the
 * routine left it before the report was added.
 */
	.section .text.fb_jump, "ax"
	.globl fb_jump
	.globl fb_jump_end
	.p2align 2

/* One byte to the port, from t1. */
	.macro	putch c
	li	t1, \c
	jal	a7, 7f
	.endm

fb_jump:
	mv	a4, a1
	mv	a6, a2
	beqz	a2, 2f
1:
	lw	t0, 0(a0)
	sw	t0, 0(a1)
	addi	a0, a0, 4
	addi	a1, a1, 4
	addi	a2, a2, -4
	bgtz	a2, 1b
	/*
	 * Stores are posted on this machine, so the last four words are
	 * read back before the jump: a load is not answered until the
	 * store to the same address is, which is measured by
	 * `a_load_behind_an_unanswered_store_to_one_address` and is what
	 * the Ethernet driver rests on too. A `fence` has waited for every
	 * posted store since issue 432 was fixed and would do the same;
	 * the read-back stays until the fence is proven on the board.
	 * Four, because the host holds at most four transactions open, one
	 * per identifier: a store further back than that had its
	 * identifier reused, so it was answered, and the last four may
	 * still be open in any order.
	 * A program shorter than four words has fewer to read, and the
	 * loop stops at its first word rather than reading below it,
	 * where nothing answers and a load would trap.
	 */
	li	t1, 4
3:
	beq	a1, a4, 2f
	beqz	t1, 2f
	addi	a1, a1, -4
	lw	t0, 0(a1)
	addi	t1, t1, -1
	j	3b
2:
	beqz	a3, 5f
	/*
	 * The sum, the words added as `fb_sum_words` adds them. They are
	 * read with `lr.w`, which goes around the data cache
	 * (issue 1556), so the destination's sum is what the DDR3 holds
	 * and not what the cache kept of the stores. The `fence` first
	 * waits for every store the copy posted, as `boot` does before its
	 * own sum of the staging area. One pass of `lr.w` over Linux is
	 * about 0.5 s (issue 2011), so the staging area is summed by
	 * `boot`, before its `booting` line, and not here.
	 */
	fence
	li	t3, 0
	mv	t5, a4
	mv	t6, a6
4:
	beqz	t6, 6f
	lr.w	t0, (t5)
	add	t3, t3, t0
	addi	t5, t5, 4
	addi	t6, t6, -4
	j	4b
6:
	putch	106	/* j */
	putch	117	/* u */
	putch	109	/* m */
	putch	112	/* p */
	putch	32	/* space */
	mv	t4, t3
	jal	a7, 8f
	putch	13	/* CR */
	putch	10	/* LF */
5:
	/*
	 * The core fetches through an instruction cache (issue 1021), which
	 * may still hold the words this image ran from before the copy
	 * replaced them: a `fence.i` empties it, so the jump fetches the
	 * program just written (issue 1211). Written as its encoding, since
	 * neither assembler that reads this file is given Zifencei.
	 */
	.insn	i 0x0f, 1, x0, x0, 0	/* fence.i */
	beqz	a3, 9f
	putch	103	/* g */
	putch	111	/* o */
	putch	13	/* CR */
	putch	10	/* LF */
	/*
	 * Room in the port's queue is not the bytes on the wire: `go` and
	 * its line end take about 350 us at 115200 baud, and the program
	 * may set the port up afresh as it starts. So the routine waits
	 * about a millisecond, 40000 turns of two instructions, first.
	 */
	li	t0, 40000
13:
	addi	t0, t0, -1
	bnez	t0, 13b
9:
	mv	a3, a4
	jr	a4

	/* t1 to the port once it has room; returns through a7. */
7:
	lw	t0, 0(a3)
	bltz	t0, 7b
	sw	t1, 0(a3)
	jr	a7

	/*
	 * t4 as eight hexadecimal digits, the highest first; returns
	 * through a7. The port is written here rather than through 7b,
	 * whose return address would take a7.
	 */
8:
	li	t5, 8
10:
	srli	t1, t4, 28
	slli	t4, t4, 4
	li	t0, 10
	blt	t1, t0, 11f
	addi	t1, t1, 39	/* 'a' - '0' - 10 */
11:
	addi	t1, t1, 48	/* '0' */
12:
	lw	t0, 0(a3)
	bltz	t0, 12b
	sw	t1, 0(a3)
	addi	t5, t5, -1
	bnez	t5, 10b
	jr	a7
fb_jump_end:
