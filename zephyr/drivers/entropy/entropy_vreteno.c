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
 *                waiting, bit 8 the repetition count test tripped,
 *                bit 9 running, bit 10 the adaptive proportion test
 *                tripped.
 *   0x08  r/w:   bit 0 run; a write with bit 1 clears the faults.
 *   0x0c  read:  the last 32 raw samples, before the extractor.
 *
 * `lib/parts/src/trng.rs` is the hardware and states the same map;
 * `cpu/vreteno/tests/zephyr_dts.rs` checks that this file and that
 * one still agree.
 *
 * The hardware's health tests are what make this driver refuse rather
 * than hand out a constant or a bias: a source whose rings have
 * stopped trips the repetition count test of SP 800-90B, one gone
 * biased trips its adaptive proportion test (issue 918), the buffer
 * stops filling, and a request here returns an error, naming the test,
 * rather than waiting for words that will not come.
 *
 * The hardware debiases with von Neumann's extractor, which removes
 * bias but not dependence, and the board's words are serially
 * correlated at lags that move between builds (issue 780). So nothing
 * read here is handed out as it is: every 32 bytes are SHA-256 over 32
 * of the extractor's words, in `vreteno_condition.c`, and a health test
 * that trips part way through refuses the whole block.
 */

#define DT_DRV_COMPAT hdlfactory_vreteno_trng

#include <zephyr/device.h>
#include <zephyr/drivers/entropy.h>
#include <zephyr/kernel.h>
#include <zephyr/logging/log.h>
/*
 * `sys_read32` and `sys_write32` are the architecture's, not the
 * generic header's, as this port's own serial driver had them before
 * issue 1011.
 */
#include <zephyr/arch/cpu.h>
#include <zephyr/sys/sys_io.h>
#include <vreteno/regs/trng.h>

#include <stdbool.h>
#include <string.h>

#include <mbedtls/platform_util.h>

#include "vreteno_condition.h"

LOG_MODULE_REGISTER(entropy_vreteno, CONFIG_ENTROPY_LOG_LEVEL);

#define VRETENO_TRNG_DATA   TRNG_DATA
#define VRETENO_TRNG_STATUS TRNG_STATUS
#define VRETENO_TRNG_CTRL   TRNG_CTRL
#define VRETENO_TRNG_RAW    TRNG_RAW

#define VRETENO_TRNG_STATUS_READY TRNG_STATUS_READY_MASK
#define VRETENO_TRNG_STATUS_FAULT TRNG_STATUS_FAULT_MASK
#define VRETENO_TRNG_STATUS_APTFAULT TRNG_STATUS_APTFAULT_MASK
#define VRETENO_TRNG_STATUS_RUN   TRNG_STATUS_RUN_MASK
#define VRETENO_TRNG_CTRL_RUN     TRNG_CTRL_RUN_MASK
#define VRETENO_TRNG_CTRL_CLEAR   TRNG_CTRL_CLEAR_MASK

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
 * none ready, and -EIO if a health test has tripped, since a source
 * in that state must not be read as if it were fine; the log says
 * which of the two (issue 918).
 */
static int trng_word(const struct device *dev, uint32_t *word)
{
	uint32_t status = trng_read(dev, VRETENO_TRNG_STATUS);

	if ((status & VRETENO_TRNG_STATUS_FAULT) != 0) {
		LOG_ERR("the repetition count test tripped");
		return -EIO;
	}
	if ((status & VRETENO_TRNG_STATUS_APTFAULT) != 0) {
		LOG_ERR("the adaptive proportion test tripped");
		return -EIO;
	}
	if ((status & VRETENO_TRNG_STATUS_READY) == 0) {
		return 0;
	}
	*word = trng_read(dev, VRETENO_TRNG_DATA);
	return 1;
}

/*
 * The blocking source of words for the conditioner: a word, or
 * -ETIMEDOUT when the source has given none in the timeout, or -EIO
 * when the health test has tripped.
 */
static int trng_next_blocking(void *ctx, uint32_t *word)
{
	const struct device *dev = ctx;
	int64_t deadline = k_uptime_ticks() + k_us_to_ticks_ceil64(
		VRETENO_TRNG_WORD_TIMEOUT_US);
	int got;

	while ((got = trng_word(dev, word)) == 0) {
		if (k_uptime_ticks() > deadline) {
			return -ETIMEDOUT;
		}
		k_yield();
	}
	return got;
}

/* The ISR's source: what is ready now, or -EAGAIN. */
struct trng_isr_ctx {
	const struct device *dev;
	bool busywait;
};

static int trng_next_isr(void *ctx, uint32_t *word)
{
	struct trng_isr_ctx *c = ctx;
	int got;

	while ((got = trng_word(c->dev, word)) == 0) {
		if (!c->busywait) {
			return -EAGAIN;
		}
	}
	return got;
}

static void trng_why(int err)
{
	/* A tripped health test is logged where it is read, in trng_word. */
	if (err == -ETIMEDOUT) {
		LOG_ERR("no word in %d us", VRETENO_TRNG_WORD_TIMEOUT_US);
	}
}

static int entropy_vreteno_get_entropy(const struct device *dev, uint8_t *buffer,
				       uint16_t length)
{
	while (length > 0) {
		uint8_t block[VRETENO_CONDITION_BYTES];
		int err = vreteno_gather(trng_next_blocking, (void *)dev, block);
		size_t n = MIN(length, sizeof(block));

		if (err != 0) {
			trng_why(err);
			return err;
		}
		memcpy(buffer, block, n);
		mbedtls_platform_zeroize(block, sizeof(block));
		buffer += n;
		length -= n;
	}
	return 0;
}

static int entropy_vreteno_get_entropy_isr(const struct device *dev, uint8_t *buffer,
					   uint16_t length, uint32_t flags)
{
	struct trng_isr_ctx ctx = {
		.dev = dev,
		.busywait = (flags & ENTROPY_BUSYWAIT) != 0,
	};
	uint16_t wanted = length;

	while (length > 0) {
		uint8_t block[VRETENO_CONDITION_BYTES];
		int err = vreteno_gather(trng_next_isr, &ctx, block);
		size_t n = MIN(length, sizeof(block));

		if (err == -EAGAIN) {
			/* Not enough words ready for a whole block, and no
			 * waiting allowed: what was gathered is dropped.
			 */
			break;
		}
		if (err != 0) {
			return err;
		}
		memcpy(buffer, block, n);
		mbedtls_platform_zeroize(block, sizeof(block));
		buffer += n;
		length -= n;
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
