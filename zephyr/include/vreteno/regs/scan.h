/* The scan register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef SCAN_REGS_H
#define SCAN_REGS_H

#define SCAN_SPAN 0x10
#define SCAN_BASE 0x00 /* read, write: the byte the next frame starts at in memory */
#define SCAN_CTRL 0x04 /* read, write: what the screen shows */
#define SCAN_CTRL_SCAN_SHIFT 0 /* read, write: the scanout when set, the framebuffer when clear */
#define SCAN_CTRL_SCAN_MASK 0x1
#define SCAN_CTRL_SCAN_WIDTH 1
#define SCAN_CTRL_SCAN_RESET 0x0
#define SCAN_STATUS 0x08 /* read only: how the scanout has kept up */
#define SCAN_STATUS_UNDER_SHIFT 0 /* read only: a column was shown before its word arrived */
#define SCAN_STATUS_UNDER_MASK 0x1
#define SCAN_STATUS_UNDER_WIDTH 1
#define SCAN_STATUS_UNDER_RESET 0x0
#define SCAN_CLEAR 0x0c /* write only: a write clears the underflow bit */

#endif
