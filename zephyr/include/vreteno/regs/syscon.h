/* The syscon register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef SYSCON_REGS_H
#define SYSCON_REGS_H

#define SYSCON_SPAN 0x20
#define SYSCON_ID 0x00 /* read only: the design's identifier, a constant */
#define SYSCON_VERSION 0x04 /* read only: its version, a constant */
#define SYSCON_STAMP0 0x08 /* read only: the build stamp, low word, a constant */
#define SYSCON_STAMP1 0x0c /* read only: the build stamp, high word, a constant */
#define SYSCON_CAUSE 0x10 /* read, write one to clear: why the system last reset; write ones to clear */
#define SYSCON_CAUSE_POWER_SHIFT 0 /* read, write one to clear: the power came on */
#define SYSCON_CAUSE_POWER_MASK 0x1
#define SYSCON_CAUSE_POWER_WIDTH 1
#define SYSCON_CAUSE_POWER_RESET 0x0
#define SYSCON_CAUSE_BUTTON_SHIFT 1 /* read, write one to clear: the button was pressed */
#define SYSCON_CAUSE_BUTTON_MASK 0x2
#define SYSCON_CAUSE_BUTTON_WIDTH 1
#define SYSCON_CAUSE_BUTTON_RESET 0x0
#define SYSCON_CAUSE_WATCHDOG_SHIFT 2 /* read, write one to clear: the watchdog gave up */
#define SYSCON_CAUSE_WATCHDOG_MASK 0x4
#define SYSCON_CAUSE_WATCHDOG_WIDTH 1
#define SYSCON_CAUSE_WATCHDOG_RESET 0x0
#define SYSCON_CAUSE_SOFTWARE_SHIFT 3 /* read, write one to clear: a program asked */
#define SYSCON_CAUSE_SOFTWARE_MASK 0x8
#define SYSCON_CAUSE_SOFTWARE_WIDTH 1
#define SYSCON_CAUSE_SOFTWARE_RESET 0x0
#define SYSCON_RESET 0x14 /* write only: written with KEY, the system resets */
#define SYSCON_SCRATCH 0x18 /* read, write: a word that survives a reset */

#endif
