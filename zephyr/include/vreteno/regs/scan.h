/* The scan register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef SCAN_REGS_H
#define SCAN_REGS_H

#define SCAN_SPAN 0x20
#define SCAN_BASE 0x00 /* read, write: the byte the next frame starts at in memory */
#define SCAN_CTRL 0x04 /* read, write: what the screen shows */
#define SCAN_CTRL_SCAN_SHIFT 0 /* read, write: the scanout when set, the framebuffer when clear */
#define SCAN_CTRL_SCAN_MASK 0x1
#define SCAN_CTRL_SCAN_WIDTH 1
#define SCAN_CTRL_SCAN_RESET 0x0
#define SCAN_CTRL_AHEAD_SHIFT 1 /* read, write: lines asked two rows ahead, from the next frame */
#define SCAN_CTRL_AHEAD_MASK 0x2
#define SCAN_CTRL_AHEAD_WIDTH 1
#define SCAN_CTRL_AHEAD_RESET 0x0
#define SCAN_STATUS 0x08 /* read only: how the scanout has kept up */
#define SCAN_STATUS_UNDER_SHIFT 0 /* read only: a column was shown before its word arrived */
#define SCAN_STATUS_UNDER_MASK 0x1
#define SCAN_STATUS_UNDER_WIDTH 1
#define SCAN_STATUS_UNDER_RESET 0x0
#define SCAN_STATUS_STUCK_SHIFT 1 /* read only: a line asked for got no word for two line times */
#define SCAN_STATUS_STUCK_MASK 0x2
#define SCAN_STATUS_STUCK_WIDTH 1
#define SCAN_STATUS_STUCK_RESET 0x0
#define SCAN_CLEAR 0x0c /* write only: a write clears the status, the longest line and the count */
#define SCAN_STUCK_AT 0x10 /* read only: the address of the line that did not come */
#define SCAN_WORST 0x14 /* read only: the longest a line took to come whole, in pixels */
#define SCAN_LATES 0x18 /* read only: how many lines had not come whole when their rows began */

#endif
