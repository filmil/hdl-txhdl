/*
 * SPDX-License-Identifier: Apache-2.0
 *
 * A source shaped like the board's, through the driver's conditioner
 * (issue 780).
 *
 *   condition_model UNCONDITIONED.log CONDITIONED.log
 *
 * The board's samples are fair and nearly independent, but carry a
 * dependence at a few lags that moves between builds: lag 19 at about
 * -0.04 on one, lags 5 and 22 at about -0.02 and +0.02 on another.
 * This makes samples with all three, from a fixed seed, runs them
 * through von Neumann's extractor as the hardware does, and writes
 * the extractor's words twice in trngstat's format: as they come, and
 * through `vreteno_condition`, 32 words to each 8. trngstat must fail
 * the first, as it fails the board's, and pass the second.
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#include "vreteno_condition.h"

/* Extractor words made; the conditioned log gets a quarter as many. */
#define WORDS 16384

/* How far back a sample may copy: past the longest lag modelled. */
#define HISTORY 32

static uint64_t state = 0x780780780780ull;

/* SplitMix64: fixed, so that the logs, and the test, never vary. */
static uint64_t rnd(void)
{
	uint64_t z = (state += 0x9e3779b97f4a7c15ull);

	z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ull;
	z = (z ^ (z >> 27)) * 0x94d049bb133111ebull;
	return z ^ (z >> 31);
}

/* True with probability p. */
static int chance(double p)
{
	return (double)(rnd() >> 11) / (double)(1ull << 53) < p;
}

/*
 * The next sample: with a small chance it repeats or inverts the one
 * a lag back, which is what a correlation at that lag is, and
 * otherwise it is a fair coin.
 */
static int sample(uint8_t hist[HISTORY], uint64_t t)
{
	int s;

	if (t >= HISTORY && chance(0.04)) {
		s = !hist[(t - 19) % HISTORY];
	} else if (t >= HISTORY && chance(0.025)) {
		s = !hist[(t - 5) % HISTORY];
	} else if (t >= HISTORY && chance(0.025)) {
		s = hist[(t - 22) % HISTORY];
	} else {
		s = (int)(rnd() & 1);
	}
	hist[t % HISTORY] = (uint8_t)s;
	return s;
}

int main(int argc, char **argv)
{
	static uint32_t words[WORDS];
	uint8_t hist[HISTORY] = { 0 };
	uint64_t t = 0;

	if (argc != 3) {
		fprintf(stderr, "usage: condition_model UNCONDITIONED CONDITIONED\n");
		return 2;
	}
	/* Von Neumann's extractor over pairs that do not overlap. */
	for (int w = 0; w < WORDS; w++) {
		uint32_t v = 0;

		for (int b = 0; b < 32;) {
			int x = sample(hist, t++);
			int y = sample(hist, t++);

			if (x != y) {
				v = (v << 1) | (uint32_t)x;
				b++;
			}
		}
		words[w] = v;
	}

	FILE *f = fopen(argv[1], "w");

	if (f == NULL) {
		perror(argv[1]);
		return 2;
	}
	fprintf(f, "trng ok\nwords\n");
	for (int w = 0; w < WORDS; w++) {
		fprintf(f, "%08x\n", words[w]);
	}
	fprintf(f, "end\n");
	fclose(f);

	f = fopen(argv[2], "w");
	if (f == NULL) {
		perror(argv[2]);
		return 2;
	}
	fprintf(f, "trng ok\nwords\n");
	for (int w = 0; w + VRETENO_CONDITION_WORDS <= WORDS;
	     w += VRETENO_CONDITION_WORDS) {
		uint8_t out[VRETENO_CONDITION_BYTES];

		if (vreteno_condition(&words[w], out) != 0) {
			fprintf(stderr, "the hash failed\n");
			return 1;
		}
		/* Words low byte first, as the driver hands bytes out. */
		for (int i = 0; i < VRETENO_CONDITION_BYTES; i += 4) {
			fprintf(f, "%08x\n",
				(uint32_t)out[i] | (uint32_t)out[i + 1] << 8 |
					(uint32_t)out[i + 2] << 16 |
					(uint32_t)out[i + 3] << 24);
		}
	}
	fprintf(f, "end\n");
	fclose(f);
	return 0;
}
