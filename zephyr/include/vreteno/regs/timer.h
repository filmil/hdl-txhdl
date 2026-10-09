/* The timer register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef TIMER_REGS_H
#define TIMER_REGS_H

#define TIMER_SPAN 0x10000
#define TIMER_MSIP 0x00 /* read, write: hart 0's software interrupt */
#define TIMER_MSIP_MSIP_SHIFT 0 /* read, write: one raises hart 0's software interrupt */
#define TIMER_MSIP_MSIP_MASK 0x1
#define TIMER_MSIP_MSIP_WIDTH 1
#define TIMER_MSIP_MSIP_RESET 0x0
#define TIMER_MSIP1 0x04 /* read, write: hart 1's software interrupt */
#define TIMER_MSIP1_MSIP1_SHIFT 0 /* read, write: one raises hart 1's software interrupt */
#define TIMER_MSIP1_MSIP1_MASK 0x1
#define TIMER_MSIP1_MSIP1_WIDTH 1
#define TIMER_MSIP1_MSIP1_RESET 0x0
#define TIMER_MTIMECMP_LO 0x4000 /* read, write: hart 0's compare, low half */
#define TIMER_MTIMECMP_HI 0x4004 /* read, write: hart 0's compare, high half */
#define TIMER_MTIMECMP1_LO 0x4008 /* read, write: hart 1's compare, low half */
#define TIMER_MTIMECMP1_HI 0x400c /* read, write: hart 1's compare, high half */
#define TIMER_MTIME_LO 0xbff8 /* read, write: the count, low half */
#define TIMER_MTIME_HI 0xbffc /* read, write: the count, high half */
#define TIMER_MBOX_ENTRY 0xc000 /* read, write: the mailbox: where hart 1 starts */
#define TIMER_MBOX_ARG 0xc004 /* read, write: the mailbox: what it starts with */

#endif
