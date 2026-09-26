/* The pwm register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef PWM_REGS_H
#define PWM_REGS_H

#define PWM_SPAN 0x20
#define PWM_CTRL 0x00 /* read, write: enable, centre, and a polarity bit per channel */
#define PWM_CTRL_ENABLE_SHIFT 0 /* read, write: the counter runs */
#define PWM_CTRL_ENABLE_MASK 0x1
#define PWM_CTRL_ENABLE_WIDTH 1
#define PWM_CTRL_ENABLE_RESET 0x0
#define PWM_CTRL_CENTRE_SHIFT 1 /* read, write: count up and down rather than up and wrap */
#define PWM_CTRL_CENTRE_MASK 0x2
#define PWM_CTRL_CENTRE_WIDTH 1
#define PWM_CTRL_CENTRE_RESET 0x0
#define PWM_CTRL_POL_SHIFT 4 /* read, write: a bit per channel: high where it would be low */
#define PWM_CTRL_POL_MASK 0xf0
#define PWM_CTRL_POL_WIDTH 4
#define PWM_CTRL_POL_RESET 0x0
#define PWM_PERIOD 0x04 /* read, write: the period, in cycles, from the next wrap */
#define PWM_DUTY0 0x08 /* read, write: channel 0: cycles high per period, from the next wrap */
#define PWM_DUTY1 0x0c /* read, write: channel 1 */
#define PWM_DUTY2 0x10 /* read, write: channel 2 */
#define PWM_DUTY3 0x14 /* read, write: channel 3 */
#define PWM_COUNT 0x18 /* read only: where in the period the counter is */

#endif
