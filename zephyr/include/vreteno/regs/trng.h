/* The trng register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef TRNG_REGS_H
#define TRNG_REGS_H

#define TRNG_SPAN 0x20
#define TRNG_DATA 0x00 /* read; the read takes it: the oldest word of entropy */
#define TRNG_STATUS 0x04 /* read only: the buffer and the health test */
#define TRNG_STATUS_READY_SHIFT 0 /* read only: a word is ready */
#define TRNG_STATUS_READY_MASK 0x1
#define TRNG_STATUS_READY_WIDTH 1
#define TRNG_STATUS_READY_RESET 0x0
#define TRNG_STATUS_COUNT_SHIFT 1 /* read only: how many words wait */
#define TRNG_STATUS_COUNT_MASK 0xe
#define TRNG_STATUS_COUNT_WIDTH 3
#define TRNG_STATUS_COUNT_RESET 0x0
#define TRNG_STATUS_FAULT_SHIFT 8 /* read only: the repetition count test tripped */
#define TRNG_STATUS_FAULT_MASK 0x100
#define TRNG_STATUS_FAULT_WIDTH 1
#define TRNG_STATUS_FAULT_RESET 0x0
#define TRNG_STATUS_RUN_SHIFT 9 /* read only: the run bit, read back */
#define TRNG_STATUS_RUN_MASK 0x200
#define TRNG_STATUS_RUN_WIDTH 1
#define TRNG_STATUS_RUN_RESET 0x0
#define TRNG_CTRL 0x08 /* read, write: the run bit, and the fault's clear */
#define TRNG_CTRL_RUN_SHIFT 0 /* read, write: the rings run and the buffer fills */
#define TRNG_CTRL_RUN_MASK 0x1
#define TRNG_CTRL_RUN_WIDTH 1
#define TRNG_CTRL_RUN_RESET 0x0
#define TRNG_CTRL_CLEAR_SHIFT 1 /* write only: written one, the fault is cleared */
#define TRNG_CTRL_CLEAR_MASK 0x2
#define TRNG_CTRL_CLEAR_WIDTH 1
#define TRNG_CTRL_CLEAR_RESET 0x0
#define TRNG_RAW 0x0c /* read only: the last 32 folded samples */
#define TRNG_CAP 0x10 /* read, write: a capture of consecutive samples */
#define TRNG_CAP_START_SHIFT 0 /* write only: written one, a capture begins */
#define TRNG_CAP_START_MASK 0x1
#define TRNG_CAP_START_WIDTH 1
#define TRNG_CAP_START_RESET 0x0
#define TRNG_CAP_BUSY_SHIFT 1 /* read only: the capture is filling */
#define TRNG_CAP_BUSY_MASK 0x2
#define TRNG_CAP_BUSY_WIDTH 1
#define TRNG_CAP_BUSY_RESET 0x0
#define TRNG_CAP_DONE_SHIFT 2 /* read only: the capture is whole */
#define TRNG_CAP_DONE_MASK 0x4
#define TRNG_CAP_DONE_WIDTH 1
#define TRNG_CAP_DONE_RESET 0x0
#define TRNG_CAPIDX 0x14 /* read, write: which captured word capword reads */
#define TRNG_CAPIDX_INDEX_SHIFT 0 /* read, write: the word, 0 to 255 */
#define TRNG_CAPIDX_INDEX_MASK 0xff
#define TRNG_CAPIDX_INDEX_WIDTH 8
#define TRNG_CAPIDX_INDEX_RESET 0x0
#define TRNG_CAPWORD 0x18 /* read only: the captured word capidx names, newest in bit 0 */

#endif
