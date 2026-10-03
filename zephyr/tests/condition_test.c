/*
 * SPDX-License-Identifier: Apache-2.0
 *
 * The entropy driver's conditioner on the host (issue 780): the hash
 * wired as SHA-256 over the words low byte first, and a gathering that
 * a health test trips part way through refused whole.
 *
 * The digests were taken with the host's `sha256sum` over the same
 * bytes, written out by `printf`, when this test was written: they are
 * not this code's own output fed back to it.
 */
#include <stdio.h>
#include <string.h>

#include "vreteno_condition.h"

static int failures;

static void check(int ok, const char *what)
{
	if (!ok) {
		printf("FAIL %s\n", what);
		failures++;
	} else {
		printf("ok   %s\n", what);
	}
}

static int digest_is(const uint8_t got[32], const char *hex)
{
	char s[65];

	for (int i = 0; i < 32; i++) {
		snprintf(&s[2 * i], 3, "%02x", got[i]);
	}
	return strcmp(s, hex) == 0;
}

/* SHA-256 of the bytes 0x00 to 0x7f. */
static const char *const KAT_SEQUENCE =
	"471fb943aa23c511f6f72f8d1652d9c880cfa392ad80503120547703e56a2be5";

/* The same with the first four bytes reversed: 03 02 01 00 04 05 .. */
static const char *const KAT_FIRST_SWAPPED =
	"39ca0580a9171b4e30a39861160fabf27af106c91da1c31a50f04c2f0c3d87af";

/* Words whose bytes, low byte first, are 0x00 to 0x7f. */
static void sequence(uint32_t w[VRETENO_CONDITION_WORDS])
{
	for (int i = 0; i < VRETENO_CONDITION_WORDS; i++) {
		w[i] = 0x03020100u + 0x04040404u * (uint32_t)i;
	}
}

/* A source that hands out `words` and fails with `error` at `fail_at`. */
struct source {
	const uint32_t *words;
	int at;
	int fail_at;
	int error;
};

static int next(void *ctx, uint32_t *word)
{
	struct source *s = ctx;

	if (s->at == s->fail_at) {
		return s->error;
	}
	*word = s->words[s->at++];
	return 1;
}

int main(void)
{
	uint32_t w[VRETENO_CONDITION_WORDS];
	uint8_t out[VRETENO_CONDITION_BYTES];

	sequence(w);
	check(vreteno_condition(w, out) == 0, "the hash runs");
	check(digest_is(out, KAT_SEQUENCE),
	      "SHA-256 over the words, low byte first");

	w[0] = 0x00010203u;
	check(vreteno_condition(w, out) == 0 && digest_is(out, KAT_FIRST_SWAPPED),
	      "a word's bytes in the other order give the other digest");

	sequence(w);
	struct source all = { w, 0, -1, 0 };
	check(vreteno_gather(next, &all, out) == 0 && digest_is(out, KAT_SEQUENCE),
	      "a gathering of 32 words is their digest");
	check(all.at == VRETENO_CONDITION_WORDS, "exactly 32 words are taken");

	/* -5 is EIO, which the driver returns when the fault bit is set. */
	struct source tripped = { w, 0, 10, -5 };
	memset(out, 0xaa, sizeof(out));
	int err = vreteno_gather(next, &tripped, out);
	int untouched = 1;
	for (int i = 0; i < VRETENO_CONDITION_BYTES; i++) {
		untouched &= out[i] == 0xaa;
	}
	check(err == -5, "a health test tripped part way is the error");
	check(untouched, "and nothing is handed out");

	struct source silent = { w, 0, 3, 0 };
	check(vreteno_gather(next, &silent, out) < 0,
	      "a source that gives neither a word nor an error is refused");

	return failures == 0 ? 0 : 1;
}
