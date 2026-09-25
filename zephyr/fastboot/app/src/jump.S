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
 *
 * Interrupts are off before this is reached, so nothing runs between
 * the copy and the jump.
 */
	.section .text.fb_jump, "ax"
	.globl fb_jump
	.globl fb_jump_end
	.p2align 2
fb_jump:
	mv	a3, a1
	beqz	a2, 2f
1:
	lw	t0, 0(a0)
	sw	t0, 0(a1)
	addi	a0, a0, 4
	addi	a1, a1, 4
	addi	a2, a2, -4
	bgtz	a2, 1b
	/*
	 * Stores are posted on this machine and `fence` does nothing
	 * (issue 432), so the last four words are read back: a load is
	 * not answered until the store to the same address is, which is
	 * measured by `a_load_behind_an_unanswered_store_to_one_address`
	 * and is what the Ethernet driver rests on too. Four, because the
	 * host holds at most four transactions open, one per identifier:
	 * a store further back than that had its identifier reused, so it
	 * was answered, and the last four may still be open in any order.
	 * A program shorter than four words has fewer to read, and the
	 * loop stops at its first word rather than reading below it,
	 * where nothing answers and a load would trap.
	 */
	li	t1, 4
3:
	beq	a1, a3, 2f
	beqz	t1, 2f
	addi	a1, a1, -4
	lw	t0, 0(a1)
	addi	t1, t1, -1
	j	3b
2:
	jr	a3
fb_jump_end:
