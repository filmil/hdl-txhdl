/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The fastboot core, fed a session in pieces (issue 143).
 *
 * Stock `fastboot` against the host harness shows the core and the
 * tool agree, but over loopback a message arrives whole, and on a
 * wire it does not: TCP hands over whatever has come, and a length
 * can be split across two segments as easily as a body can. So the
 * same session is fed here one byte at a time, in threes, in sevens,
 * in segment-sized pieces and all at once, and every one must give
 * the same answers and stage the same bytes.
 */
#include "fastboot.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define STAGE 4096

static uint8_t stage[STAGE];
static uint8_t sent[65536];
static size_t sent_len;

static int t_write(void *ctx, uint32_t off, const uint8_t *p, uint32_t n)
{
	(void)ctx;
	if (off + n > STAGE) {
		return 1;
	}
	memcpy(stage + off, p, n);
	return 0;
}

static int t_send(void *ctx, const uint8_t *p, size_t n)
{
	(void)ctx;
	memcpy(sent + sent_len, p, n);
	sent_len += n;
	return 0;
}

static const struct fb_ops ops = {
	.max_download = STAGE,
	.write = t_write,
	.send = t_send,
};

/* A message as the host frames one: an eight byte big-endian length,
 * then the bytes. */
static size_t frame(uint8_t *out, const void *p, size_t n)
{
	for (int i = 0; i < 8; i++) {
		out[i] = (uint8_t)((uint64_t)n >> (56 - 8 * i));
	}
	memcpy(out + 8, p, n);
	return 8 + n;
}

static int failures;

static void expect(int ok, const char *what, size_t piece)
{
	if (!ok) {
		printf("FAIL in pieces of %zu: %s\n", piece, what);
		failures++;
	}
}

/* The answers the session should get, framed, in order. */
static size_t expected(uint8_t *out)
{
	size_t n = 0;

	memcpy(out, "FB01", 4);
	n += 4;
	n += frame(out + n, "OKAY0x00001000", 14);
	n += frame(out + n, "FAILunknown variable", 20);
	n += frame(out + n, "DATA00000400", 12);
	n += frame(out + n, "OKAY", 4);
	n += frame(out + n, "FAILthis machine has no storage to write", 40);
	n += frame(out + n, "OKAY", 4);
	return n;
}

int main(void)
{
	static uint8_t session[8192];
	static uint8_t image[1024];
	static uint8_t want[1024];
	size_t n = 0;

	for (size_t i = 0; i < sizeof(image); i++) {
		image[i] = (uint8_t)(i * 7 + 3);
	}
	memcpy(session, "FB01", 4);
	n += 4;
	n += frame(session + n, "getvar:max-download-size", 24);
	n += frame(session + n, "getvar:is-userspace", 19);
	n += frame(session + n, "download:00000400", 17);
	/* The image in three messages of uneven size, as a host may send
	 * a download in more than one write. */
	n += frame(session + n, image, 100);
	n += frame(session + n, image + 100, 900);
	n += frame(session + n, image + 1000, 24);
	n += frame(session + n, "flash:boot", 10);
	n += frame(session + n, "boot", 4);

	size_t want_len = expected(want);
	const size_t pieces[] = {1, 3, 7, 1500, sizeof(session)};

	for (size_t k = 0; k < sizeof(pieces) / sizeof(pieces[0]); k++) {
		struct fb fb;
		size_t piece = pieces[k];
		enum fb_result r = FB_MORE;
		size_t at = 0;

		memset(stage, 0, sizeof(stage));
		sent_len = 0;
		fb_init(&fb, &ops);
		while (at < n && r == FB_MORE) {
			size_t take = n - at < piece ? n - at : piece;

			r = fb_input(&fb, session + at, take);
			at += take;
		}
		expect(r == FB_BOOT, "the session ends in boot", piece);
		expect(at == n, "every byte was taken", piece);
		expect(sent_len == want_len &&
			       memcmp(sent, want, want_len) == 0,
		       "the answers", piece);
		expect(memcmp(stage, image, sizeof(image)) == 0,
		       "the staged bytes", piece);
		expect(fb.staged == sizeof(image), "the staged size", piece);
	}

	/* Refusals: a peer that is not fastboot, a download too large, a
	 * boot with nothing staged, and a command longer than allowed. */
	{
		struct fb fb;

		fb_init(&fb, &ops);
		expect(fb_input(&fb, (const uint8_t *)"GET / HTTP", 10) ==
			       FB_CLOSE,
		       "a peer that is not fastboot is closed", 0);
	}
	{
		struct fb fb;
		uint8_t m[64];
		size_t k = 0;

		sent_len = 0;
		fb_init(&fb, &ops);
		memcpy(m, "FB01", 4);
		k = 4 + frame(m + 4, "download:00001001", 17);
		expect(fb_input(&fb, m, k) == FB_MORE, "a large download", 0);
		expect(sent_len == 4 + 8 + 21 &&
			       memcmp(sent + 12, "FAILsize out of range", 21) ==
				       0,
		       "a download larger than max-download-size fails", 0);
		sent_len = 0;
		k = frame(m, "boot", 4);
		expect(fb_input(&fb, m, k) == FB_MORE &&
			       memcmp(sent + 8, "FAILnothing downloaded", 22) ==
				       0,
		       "boot with nothing staged fails", 0);
		/* A platform that cannot reset says so rather than answer
		 * `OKAY` to a reset that will not come. */
		sent_len = 0;
		k = frame(m, "reboot", 6);
		expect(fb_input(&fb, m, k) == FB_MORE &&
			       memcmp(sent + 8, "FAILthis machine cannot reset",
				      29) == 0,
		       "reboot on a machine that cannot reset fails", 0);
	}
	{
		struct fb fb;
		uint8_t m[12] = {'F', 'B', '0', '1', 0, 0, 0, 0, 0, 0, 0x10, 0x01};

		fb_init(&fb, &ops);
		expect(fb_input(&fb, m, sizeof(m)) == FB_CLOSE,
		       "a command longer than 4096 bytes is closed", 0);
	}

	/* The boot image the host wraps a file in: kernel size at 8,
	 * page size at 36, the kernel at the page. */
	{
		static uint8_t img[4096 + 16];
		uint32_t off, size;

		memset(img, 0, sizeof(img));
		memcpy(img, "ANDROID!", 8);
		img[8] = 16;
		img[36] = 0x00;
		img[37] = 0x10;
		expect(fb_kernel(img, sizeof(img), &off, &size) == 0 &&
			       off == 4096 && size == 16,
		       "the kernel is found at the page size", 0);
		img[8] = 17;
		expect(fb_kernel(img, sizeof(img), &off, &size) != 0,
		       "a kernel longer than the image is refused", 0);
		expect(fb_kernel(image, sizeof(image), &off, &size) == 0 &&
			       off == 0 && size == sizeof(image),
		       "an image with no header is the program", 0);
	}

	if (failures) {
		printf("%d failures\n", failures);
		return 1;
	}
	printf("all pieces agree\n");
	return 0;
}
