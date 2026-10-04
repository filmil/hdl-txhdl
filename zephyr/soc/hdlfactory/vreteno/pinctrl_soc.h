/* SPDX-License-Identifier: Apache-2.0 */
/*
 * Zephyr's `drivers/pinctrl.h` includes this from every SoC, and the
 * SiFive serial driver includes that (issue 1011). This board has no
 * pin multiplexer: every pin is wired to its one function in the
 * bitstream. So a pin control state is never declared, and these are
 * the type and the two macros the header needs to compile, describing
 * nothing.
 */
#ifndef VRETENO_PINCTRL_SOC_H_
#define VRETENO_PINCTRL_SOC_H_

#include <zephyr/types.h>

typedef struct pinctrl_soc_pin_t {
	uint8_t unused;
} pinctrl_soc_pin_t;

#define Z_PINCTRL_STATE_PIN_INIT(node_id, prop, idx) {0},

#define Z_PINCTRL_STATE_PINS_INIT(node_id, prop)		\
	{ DT_FOREACH_PROP_ELEM(node_id, prop, Z_PINCTRL_STATE_PIN_INIT) }

#endif /* VRETENO_PINCTRL_SOC_H_ */
