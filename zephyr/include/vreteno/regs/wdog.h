/* The wdog register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef WDOG_REGS_H
#define WDOG_REGS_H

#define WDOG_SPAN 0x20
#define WDOG_CTRL 0x00 /* read, write: enable, window, warn and lock */
#define WDOG_CTRL_ENABLE_SHIFT 0 /* read, write: the watchdog counts */
#define WDOG_CTRL_ENABLE_MASK 0x1
#define WDOG_CTRL_ENABLE_WIDTH 1
#define WDOG_CTRL_ENABLE_RESET 0x0
#define WDOG_CTRL_WINDOW_SHIFT 1 /* read, write: a refresh above sill is a failure */
#define WDOG_CTRL_WINDOW_MASK 0x2
#define WDOG_CTRL_WINDOW_WIDTH 1
#define WDOG_CTRL_WINDOW_RESET 0x0
#define WDOG_CTRL_WARN_SHIFT 2 /* read, write: the first timeout warns instead of resetting */
#define WDOG_CTRL_WARN_MASK 0x4
#define WDOG_CTRL_WARN_WIDTH 1
#define WDOG_CTRL_WARN_RESET 0x0
#define WDOG_CTRL_LOCK_SHIFT 3 /* read, write: set, ctrl, load and sill are read only */
#define WDOG_CTRL_LOCK_MASK 0x8
#define WDOG_CTRL_LOCK_WIDTH 1
#define WDOG_CTRL_LOCK_RESET 0x0
#define WDOG_LOAD 0x04 /* read, write: the timeout a refresh loads */
#define WDOG_LOAD_CYCLES_SHIFT 0 /* read, write: the timeout, in cycles */
#define WDOG_LOAD_CYCLES_MASK 0xffff
#define WDOG_LOAD_CYCLES_WIDTH 16
#define WDOG_LOAD_CYCLES_RESET 0x0
#define WDOG_COUNT 0x08 /* read only: what is left of the timeout */
#define WDOG_COUNT_CYCLES_SHIFT 0 /* read only: what is left, in cycles */
#define WDOG_COUNT_CYCLES_MASK 0xffff
#define WDOG_COUNT_CYCLES_WIDTH 16
#define WDOG_COUNT_CYCLES_RESET 0x0
#define WDOG_FEED 0x0c /* write only: written with KEY: software is still there */
#define WDOG_STATUS 0x10 /* read, write one to clear: warned and failed; write ones to clear */
#define WDOG_STATUS_WARNED_SHIFT 0 /* read, write one to clear: the warning timeout has happened */
#define WDOG_STATUS_WARNED_MASK 0x1
#define WDOG_STATUS_WARNED_WIDTH 1
#define WDOG_STATUS_WARNED_RESET 0x0
#define WDOG_STATUS_FAILED_SHIFT 1 /* read, write one to clear: the watchdog asked for a reset */
#define WDOG_STATUS_FAILED_MASK 0x2
#define WDOG_STATUS_FAILED_WIDTH 1
#define WDOG_STATUS_FAILED_RESET 0x0
#define WDOG_SILL 0x14 /* read, write: the window opens when count falls to this */
#define WDOG_SILL_CYCLES_SHIFT 0 /* read, write: the sill, in cycles */
#define WDOG_SILL_CYCLES_MASK 0xffff
#define WDOG_SILL_CYCLES_WIDTH 16
#define WDOG_SILL_CYCLES_RESET 0x0

#endif
