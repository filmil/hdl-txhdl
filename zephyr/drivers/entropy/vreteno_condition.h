/*
 * SPDX-License-Identifier: Apache-2.0
 *
 * The conditioner of the Vreteno entropy source (issue 780): SHA-256,
 * from Zephyr's Mbed TLS, over 32 of the extractor's words for each 32
 * bytes handed out.
 *
 * The extractor's words are serially correlated on the board, at lags
 * that move from one build to the next, so no fixed fold removes the
 * dependence; a hash that mixes every input bit into every output bit
 * does. The ratio is four input bits to each output bit: SP 800-90B,
 * section 3.1.5.1.2, lets a vetted conditioner's 256-bit output claim
 * full entropy when its input carries at least 256 + 64 bits, and 1024
 * input bits do so if each carries half a bit, against the 0.99 per bit
 * the board's captures measure by the most common value. Half a bit is
 * an engineering margin for the correlation, not an assessment.
 *
 * Plain C over Mbed TLS and nothing of Zephyr's, so that a host test
 * builds the same file (zephyr/tests/condition_test.c).
 */
#ifndef VRETENO_CONDITION_H_
#define VRETENO_CONDITION_H_

#include <stdint.h>

/* The extractor's words that go into one output. */
#define VRETENO_CONDITION_WORDS 32

/* The bytes one output gives: a SHA-256 digest. */
#define VRETENO_CONDITION_BYTES 32

/*
 * The digest of `in`, its words taken low byte first, into `out`.
 * Returns 0, or Mbed TLS's error.
 */
int vreteno_condition(const uint32_t in[VRETENO_CONDITION_WORDS],
		      uint8_t out[VRETENO_CONDITION_BYTES]);

/*
 * The source of words for `vreteno_gather`: 1 with a word in `*word`,
 * or a negative error, which ends the gathering. A source that waits
 * for its words does so inside, and says so with an error when it
 * gives up.
 */
typedef int (*vreteno_next_word)(void *ctx, uint32_t *word);

/*
 * Gathers VRETENO_CONDITION_WORDS words from `next` and conditions
 * them into `out`. Returns 0, or the first error `next` gave, in which
 * case nothing is written to `out`: a health test that tripped part
 * way through a gathering refuses the whole of it. The words are wiped
 * either way.
 */
int vreteno_gather(vreteno_next_word next, void *ctx,
		   uint8_t out[VRETENO_CONDITION_BYTES]);

#endif /* VRETENO_CONDITION_H_ */
