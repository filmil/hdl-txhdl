/* SPDX-License-Identifier: Apache-2.0 */
/*
 * What Zephyr's SiFive drivers ask of a SoC that uses them: the clock
 * the peripherals run on. The serial port is SiFive's (issue 1011), and
 * `uart_sifive.c` divides this into the baud rate; it is the device
 * tree's `pclk`, the memory controller's 100 MHz.
 */
#ifndef VRETENO_SOC_H_
#define VRETENO_SOC_H_

#define SIFIVE_PERIPHERAL_CLOCK_FREQUENCY \
	DT_PROP(DT_NODELABEL(pclk), clock_frequency)

#endif /* VRETENO_SOC_H_ */
