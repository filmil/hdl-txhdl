/*
 * SPDX-License-Identifier: Apache-2.0
 *
 * The entropy driver's output, conditioned, on the serial port (issue
 * 780). It prints `trng ok`, `words` and 4096 words of what the driver
 * hands out, eight hex digits a line, low byte first as the driver
 * fills the buffer, then `end`: the format `//tools/trngstat` reads.
 * A refusal prints `fault` and the error, which trngstat reports as a
 * capture stopped on a fault.
 */
#include <zephyr/device.h>
#include <zephyr/drivers/entropy.h>
#include <zephyr/sys/printk.h>

#define WORDS 4096

int main(void)
{
	const struct device *dev = DEVICE_DT_GET_ONE(hdlfactory_vreteno_trng);

	if (!device_is_ready(dev)) {
		printk("fault\nno entropy device\n");
		return 0;
	}
	printk("trng ok\nwords\n");
	for (int w = 0; w < WORDS; w += 8) {
		uint8_t block[32];
		int err = entropy_get_entropy(dev, block, sizeof(block));

		if (err != 0) {
			printk("fault\nerror %d\n", err);
			return 0;
		}
		for (int i = 0; i < 32; i += 4) {
			printk("%08x\n", (uint32_t)block[i] |
				(uint32_t)block[i + 1] << 8 |
				(uint32_t)block[i + 2] << 16 |
				(uint32_t)block[i + 3] << 24);
		}
	}
	printk("end\n");
	return 0;
}
