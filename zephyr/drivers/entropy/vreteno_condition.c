/*
 * SPDX-License-Identifier: Apache-2.0
 *
 * The conditioner of the Vreteno entropy source; see the header.
 */
#include "vreteno_condition.h"

#include <mbedtls/platform_util.h>
#include <mbedtls/sha256.h>

int vreteno_condition(const uint32_t in[VRETENO_CONDITION_WORDS],
		      uint8_t out[VRETENO_CONDITION_BYTES])
{
	uint8_t bytes[VRETENO_CONDITION_WORDS * 4];
	int err;

	for (int i = 0; i < VRETENO_CONDITION_WORDS; i++) {
		bytes[4 * i + 0] = (uint8_t)(in[i] >> 0);
		bytes[4 * i + 1] = (uint8_t)(in[i] >> 8);
		bytes[4 * i + 2] = (uint8_t)(in[i] >> 16);
		bytes[4 * i + 3] = (uint8_t)(in[i] >> 24);
	}
	/* The last argument 0 is SHA-256; 1 would be SHA-224. */
	err = mbedtls_sha256(bytes, sizeof(bytes), out, 0);
	mbedtls_platform_zeroize(bytes, sizeof(bytes));
	return err;
}

int vreteno_gather(vreteno_next_word next, void *ctx,
		   uint8_t out[VRETENO_CONDITION_BYTES])
{
	uint32_t words[VRETENO_CONDITION_WORDS];
	int err = 0;

	for (int i = 0; i < VRETENO_CONDITION_WORDS; i++) {
		int got = next(ctx, &words[i]);

		/* Anything but a word ends it; a source that says neither
		 * a word nor an error has not given one.
		 */
		if (got != 1) {
			err = got < 0 ? got : -1;
			break;
		}
	}
	if (err == 0) {
		err = vreteno_condition(words, out);
	}
	mbedtls_platform_zeroize(words, sizeof(words));
	return err;
}
