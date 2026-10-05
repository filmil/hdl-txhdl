/* The doorbell register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef DOORBELL_REGS_H
#define DOORBELL_REGS_H

#define DOORBELL_SPAN 0x8
#define DOORBELL_COUNT 0x00 /* read, write: the entries to draw; zero when there is nothing to draw */
#define DOORBELL_COUNT_COUNT_SHIFT 0 /* read, write: the count */
#define DOORBELL_COUNT_COUNT_MASK 0xffff
#define DOORBELL_COUNT_COUNT_WIDTH 16
#define DOORBELL_COUNT_COUNT_RESET 0x0
#define DOORBELL_STATUS 0x04 /* read only: what the rasteriser is doing */
#define DOORBELL_STATUS_IDLE_SHIFT 0 /* read only: no list is being drawn */
#define DOORBELL_STATUS_IDLE_MASK 0x1
#define DOORBELL_STATUS_IDLE_WIDTH 1
#define DOORBELL_STATUS_IDLE_RESET 0x1

#endif
