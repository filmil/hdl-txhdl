/* The timer register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef TIMER_REGS_H
#define TIMER_REGS_H

#define TIMER_SPAN 0x10000
#define TIMER_MSIP 0x00 /* read, write: the software interrupt */
#define TIMER_MSIP_MSIP_SHIFT 0 /* read, write: one raises the software interrupt */
#define TIMER_MSIP_MSIP_MASK 0x1
#define TIMER_MSIP_MSIP_WIDTH 1
#define TIMER_MSIP_MSIP_RESET 0x0
#define TIMER_MTIMECMP_LO 0x4000 /* read, write: the compare, low half */
#define TIMER_MTIMECMP_HI 0x4004 /* read, write: the compare, high half */
#define TIMER_MTIME_LO 0xbff8 /* read, write: the count, low half */
#define TIMER_MTIME_HI 0xbffc /* read, write: the count, high half */

#endif
