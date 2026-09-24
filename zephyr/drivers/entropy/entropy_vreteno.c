/*
 * SPDX-License-Identifier: Apache-2.0
 *
 * The Vreteno entropy source: ring oscillators sampled, debiased and
 * buffered by the hardware, read here a word at a time.
 *
 * The registers are four words:
 *
 *   0x00  read:  a word of entropy, taken by the read; zero if none.
 *   0x04  read:  bit 0 a word is ready, bits 3 to 1 how many are
 *                waiting, bit 8 the health test tripped, bit 9 running.
 *   0x08  r/w:   bit 0 run; a write with bit 1 clears the fault.
 *   0x0c  read:  the last 32 raw samples, before the extractor.
 *
 * `lib/parts/src/trng.rs` is the hardware and states the same map;
 * `cpu/vreteno/tests/zephyr_dts.rs` checks that this file and that
 * one still agree.
 *
 * The hardware's health test is what makes this driver refuse rather
 * than hand out a constant: a source whose rings have stopped trips
 * the test, the buffer stops filling, and a request here returns an
 * error rather than waiting for words that will not come. Nothing is
 * mixed in on this side: the hardware debiases with von Neumann's
 * extractor, and what is read is what it kept.
 */

#define DT_DRV_COMPAT hdlfactory_vreteno_trng

#include <zephyr/device.h>
#include <zephyr/drivers/entropy.h>
#include <zephyr/kernel.h>
#include <zephyr/logging/log.h>
/*
 * `sys_read32` and `sys_write32` are the architecture's, not the
 * generic header's, exactly as in `uart_vreteno.c`.
 */
#include <zephyr/arch/cpu.h>
#include <zephyr/sys/sys_io.h>

LOG_MODULE_REGISTER(entropy_vreteno, CONFIG_ENTROPY_LOG_LEVEL);

#define VRETENO_TRNG_DATA   0x00
#define VRETENO_TRNG_STATUS 0x04
#define VRETENO_TRNG_CTRL   0x08
#define VRETENO_TRNG_RAW    0x0c

#define VRETENO_TRNG_STATUS_READY BIT(0)
#define VRETENO_TRNG_STATUS_FAULT BIT(8)
#define VRETENO_TRNG_STATUS_RUN   BIT(9)
#define VRETENO_TRNG_CTRL_RUN     BIT(0)
#define VRETENO_TRNG_CTRL_CLEAR   BIT(1)

/*
 * How long a blocking read waits for one word before giving up. A
 * word takes about 130 cycles of the source's clock to gather, so a
 * source that has not produced one in a millisecond is not producing.
 */
#define VRETENO_TRNG_WORD_TIMEOUT_US 1000

struct entropy_vreteno_config {
	mm_reg_t base;
};

static inline uint32_t trng_read(const struct device *dev, uint32_t off)
{
	const struct entropy_vreteno_config *config = dev->config;

	return sys_read32(config->base + off);
}

static inline void trng_write(const struct device *dev, uint32_t off, uint32_t v)
{
	const struct entropy_vreteno_config *config = dev->config;

	sys_write32(v, config->base + off);
}

/*
 * One word, if the hardware has one. Returns 1 with the word, 0 with
 * none ready, and -EIO if the health test has tripped, since a source
 * in that state must not be read as if it were fine.
 */
static int trng_word(const struct device *dev, uint32_t *word)
{
	uint32_t status = trng_read(dev, VRETENO_TRNG_STATUS);

	if ((status & VRETENO_TRNG_STATUS_FAULT) != 0) {
		return -EIO;
	}
	if ((status & VRETENO_TRNG_STATUS_READY) == 0) {
		return 0;
	}
	*word = trng_read(dev, VRETENO_TRNG_DATA);
	return 1;
}

static void trng_put(uint8_t **buffer, uint16_t *length, uint32_t word)
{
	size_t n = MIN(*length, sizeof(word));

	memcpy(*buffer, &word, n);
	*buffer += n;
	*length -= n;
}

static int entropy_vreteno_get_entropy(const struct device *dev, uint8_t *buffer,
				       uint16_t length)
{
	while (length > 0) {
		uint32_t word;
		int64_t deadline = k_uptime_ticks() + k_us_to_ticks_ceil64(
			VRETENO_TRNG_WORD_TIMEOUT_US);
		int got;

		while ((got = trng_word(dev, &word)) == 0) {
			if (k_uptime_ticks() > deadline) {
				LOG_ERR("no word in %d us", VRETENO_TRNG_WORD_TIMEOUT_US);
				return -ETIMEDOUT;
			}
			k_yield();
		}
		if (got < 0) {
			LOG_ERR("the health test tripped");
			return got;
		}
		trng_put(&buffer, &length, word);
	}
	return 0;
}

static int entropy_vreteno_get_entropy_isr(const struct device *dev, uint8_t *buffer,
					   uint16_t length, uint32_t flags)
{
	uint16_t wanted = length;

	while (length > 0) {
		uint32_t word;
		int got = trng_word(dev, &word);

		if (got < 0) {
			return got;
		}
		if (got == 0) {
			if ((flags & ENTROPY_BUSYWAIT) == 0) {
				break;
			}
			continue;
		}
		trng_put(&buffer, &length, word);
	}
	return wanted - length;
}

static int entropy_vreteno_init(const struct device *dev)
{
	/* Start the rings, and clear whatever a reset left in the fault. */
	trng_write(dev, VRETENO_TRNG_CTRL, VRETENO_TRNG_CTRL_RUN | VRETENO_TRNG_CTRL_CLEAR);
	return 0;
}

static DEVICE_API(entropy, entropy_vreteno_api) = {
	.get_entropy = entropy_vreteno_get_entropy,
	.get_entropy_isr = entropy_vreteno_get_entropy_isr,
};

#define ENTROPY_VRETENO_INIT(n)                                                    \
	static const struct entropy_vreteno_config entropy_vreteno_##n##_config = {  \
		.base = DT_INST_REG_ADDR(n),                                       \
	};                                                                         \
                                                                                   \
	DEVICE_DT_INST_DEFINE(n, entropy_vreteno_init, NULL, NULL,                 \
			      &entropy_vreteno_##n##_config, PRE_KERNEL_1,         \
			      CONFIG_ENTROPY_INIT_PRIORITY, &entropy_vreteno_api);

DT_INST_FOREACH_STATUS_OKAY(ENTROPY_VRETENO_INIT)
