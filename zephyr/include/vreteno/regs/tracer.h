/* The tracer register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef TRACER_REGS_H
#define TRACER_REGS_H

#define TRACER_SPAN 0x20
#define TRACER_CTRL 0x00 /* read, write: run, and freeze when halt rises */
#define TRACER_CTRL_RUN_SHIFT 0 /* read, write: the buffer takes what it is offered */
#define TRACER_CTRL_RUN_MASK 0x1
#define TRACER_CTRL_RUN_WIDTH 1
#define TRACER_CTRL_RUN_RESET 0x0
#define TRACER_CTRL_FREEZE_SHIFT 1 /* read, write: the first cycle of halt clears run */
#define TRACER_CTRL_FREEZE_MASK 0x2
#define TRACER_CTRL_FREEZE_WIDTH 1
#define TRACER_CTRL_FREEZE_RESET 0x0
#define TRACER_COUNT 0x04 /* read only: entries held */
#define TRACER_COUNT_ENTRIES_SHIFT 0 /* read only: how many, up to N */
#define TRACER_COUNT_ENTRIES_MASK 0xffff
#define TRACER_COUNT_ENTRIES_WIDTH 16
#define TRACER_COUNT_ENTRIES_RESET 0x0
#define TRACER_CURSOR 0x08 /* read, write: which held entry a read returns */
#define TRACER_CURSOR_ENTRY_SHIFT 0 /* read, write: counting from the oldest */
#define TRACER_CURSOR_ENTRY_MASK 0xffff
#define TRACER_CURSOR_ENTRY_WIDTH 16
#define TRACER_CURSOR_ENTRY_RESET 0x0
#define TRACER_WORD0 0x10 /* read only: bits 31 to 0 of the entry the cursor names */
#define TRACER_WORD1 0x14 /* read only: bits 63 to 32 */
#define TRACER_WORD2 0x18 /* read only: bits 95 to 64 */
#define TRACER_WORD3 0x1c /* read only: bits 127 to 96 */

#endif
