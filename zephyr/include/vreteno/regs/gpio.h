/* The gpio register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef GPIO_REGS_H
#define GPIO_REGS_H

#define GPIO_SPAN 0x20
#define GPIO_OUT 0x00 /* read, write: driven on a pin whose direction is out */
#define GPIO_PINS 0x04 /* read only: what the pins read, after the two flip-flops */
#define GPIO_DIR 0x08 /* read, write: one to drive the pin, zero to read it */
#define GPIO_IE 0x0c /* read, write: one to let the pin raise the interrupt */
#define GPIO_KIND 0x10 /* read, write: zero for a level, one for an edge */
#define GPIO_POL 0x14 /* read, write: a level: one for high; an edge: one for rising */
#define GPIO_STATUS 0x18 /* read, write one to clear: which pins have fired */

#endif
