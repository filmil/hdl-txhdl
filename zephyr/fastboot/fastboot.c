/* SPDX-License-Identifier: Apache-2.0 */
/*
 * Fastboot over TCP, the device's side; see `fastboot.h`.
 *
 * Written from AOSP's `fastboot/README.md` and checked against the
 * stock host tool by `fastboot_test.sh`. u-boot's `net/fastboot_tcp.c`
 * was read for what the specification leaves implicit, chiefly that a
 * message can arrive in any number of pieces, and nothing here is
 * taken from it, since it is GPL.
 */
#include "fastboot.h"

#include <string.h>

enum {
	PHASE_HANDSHAKE,
	PHASE_LENGTH,
	PHASE_BODY,
};

void fb_init(struct fb *fb, const struct fb_ops *ops)
{
	memset(fb, 0, sizeof(*fb));
	fb->ops = ops;
	fb->phase = PHASE_HANDSHAKE;
}

/* One answer, as a message: its eight byte length, then the tag and
 * the text. An answer is short, so it is built whole and sent once. */
static int answer(struct fb *fb, const char *tag, const char *text)
{
	uint8_t buf[8 + 4 + 256];
	size_t n = strlen(text);

	if (n > 256) {
		n = 256;
	}
	uint64_t len = 4 + n;

	for (int i = 0; i < 8; i++) {
		buf[i] = (uint8_t)(len >> (56 - 8 * i));
	}
	memcpy(buf + 8, tag, 4);
	memcpy(buf + 12, text, n);
	return fb->ops->send(fb->ops->ctx, buf, 12 + n);
}

/* `value` as eight lower-case hexadecimal digits, which is how
 * `download` states a size and how `DATA` repeats it. */
static void hex8(char *out, uint32_t value)
{
	static const char digits[] = "0123456789abcdef";

	for (int i = 0; i < 8; i++) {
		out[i] = digits[(value >> (28 - 4 * i)) & 0xf];
	}
	out[8] = '\0';
}

/* Eight hexadecimal digits, either case, as `download:%08x` sends them.
 * Returns nonzero for anything else. */
static int parse_hex8(const char *s, uint32_t *value)
{
	uint32_t v = 0;

	if (strlen(s) != 8) {
		return 1;
	}
	for (int i = 0; i < 8; i++) {
		char c = s[i];
		uint32_t d;

		if (c >= '0' && c <= '9') {
			d = (uint32_t)(c - '0');
		} else if (c >= 'a' && c <= 'f') {
			d = (uint32_t)(c - 'a' + 10);
		} else if (c >= 'A' && c <= 'F') {
			d = (uint32_t)(c - 'A' + 10);
		} else {
			return 1;
		}
		v = (v << 4) | d;
	}
	*value = v;
	return 0;
}

static int starts(const char *s, const char *prefix)
{
	return strncmp(s, prefix, strlen(prefix)) == 0;
}

static enum fb_result getvar(struct fb *fb, const char *name)
{
	char value[64];

	if (strcmp(name, "version") == 0) {
		/* The protocol's version, which the README gives as 0.4. */
		return answer(fb, "OKAY", "0.4") ? FB_CLOSE : FB_MORE;
	}
	if (strcmp(name, "max-download-size") == 0) {
		value[0] = '0';
		value[1] = 'x';
		hex8(value + 2, fb->ops->max_download);
		return answer(fb, "OKAY", value) ? FB_CLOSE : FB_MORE;
	}
	if (fb->ops->getvar &&
	    fb->ops->getvar(fb->ops->ctx, name, value, sizeof(value)) == 0) {
		value[sizeof(value) - 1] = '\0';
		return answer(fb, "OKAY", value) ? FB_CLOSE : FB_MORE;
	}
	/* The host asks for variables a phone has and this machine does
	 * not, such as `is-userspace`, and takes `FAIL` as "no". */
	return answer(fb, "FAIL", "unknown variable") ? FB_CLOSE : FB_MORE;
}

static enum fb_result command(struct fb *fb, const char *cmd)
{
	uint32_t size;
	char text[9];

	if (starts(cmd, "getvar:")) {
		return getvar(fb, cmd + strlen("getvar:"));
	}
	if (starts(cmd, "download:")) {
		if (parse_hex8(cmd + strlen("download:"), &size)) {
			return answer(fb, "FAIL", "bad size") ? FB_CLOSE : FB_MORE;
		}
		if (size == 0 || size > fb->ops->max_download) {
			return answer(fb, "FAIL", "size out of range") ? FB_CLOSE
								       : FB_MORE;
		}
		fb->downloading = 1;
		fb->want = size;
		fb->got = 0;
		fb->staged = 0;
		hex8(text, size);
		return answer(fb, "DATA", text) ? FB_CLOSE : FB_MORE;
	}
	if (strcmp(cmd, "boot") == 0) {
		if (fb->staged == 0) {
			return answer(fb, "FAIL", "nothing downloaded") ? FB_CLOSE
									: FB_MORE;
		}
		return answer(fb, "OKAY", "") ? FB_CLOSE : FB_BOOT;
	}
	if (strcmp(cmd, "reboot") == 0) {
		if (!fb->ops->can_reboot) {
			return answer(fb, "FAIL", "this machine cannot reset itself")
				       ? FB_CLOSE
				       : FB_MORE;
		}
		return answer(fb, "OKAY", "") ? FB_CLOSE : FB_REBOOT;
	}
	if (starts(cmd, "flash:") || starts(cmd, "erase:")) {
		return answer(fb, "FAIL", "this machine has no storage to write")
			       ? FB_CLOSE
			       : FB_MORE;
	}
	return answer(fb, "FAIL", "unknown command") ? FB_CLOSE : FB_MORE;
}

/* The bytes of a message's body, which are either a command being
 * gathered or, between `DATA` and the last byte, the download. Returns
 * how many of `n` it took. */
static size_t body(struct fb *fb, const uint8_t *p, size_t n,
		   enum fb_result *result)
{
	uint64_t left = fb->msg_len - fb->msg_got;
	size_t take = n < left ? n : (size_t)left;

	if (fb->downloading) {
		/* A message carries no more than the download asked for; a
		 * host that sends more is not speaking fastboot. */
		if (fb->got + take > fb->want) {
			*result = FB_CLOSE;
			return take;
		}
		if (fb->ops->write(fb->ops->ctx, fb->got, p, (uint32_t)take)) {
			fb->downloading = 0;
			*result = answer(fb, "FAIL", "could not store the image")
					  ? FB_CLOSE
					  : FB_MORE;
			return take;
		}
		fb->got += (uint32_t)take;
	} else {
		memcpy(fb->cmd + fb->msg_got, p, take);
	}
	fb->msg_got += take;
	if (fb->msg_got < fb->msg_len) {
		return take;
	}

	/* The message is whole. */
	fb->phase = PHASE_LENGTH;
	fb->head_len = 0;
	if (fb->downloading) {
		if (fb->got == fb->want) {
			fb->downloading = 0;
			fb->staged = fb->want;
			*result = answer(fb, "OKAY", "") ? FB_CLOSE : FB_MORE;
		}
		return take;
	}
	fb->cmd[fb->msg_len] = '\0';
	*result = command(fb, fb->cmd);
	return take;
}

enum fb_result fb_input(struct fb *fb, const uint8_t *p, size_t n)
{
	enum fb_result result = FB_MORE;

	while (n > 0 && result == FB_MORE) {
		switch (fb->phase) {
		case PHASE_HANDSHAKE:
			fb->head[fb->head_len++] = *p++;
			n--;
			if (fb->head_len < 4) {
				break;
			}
			/* `FB` and a two digit version. Every version the
			 * host could send is at least 1, and this side
			 * speaks 1, so the minimum is 1. */
			if (fb->head[0] != 'F' || fb->head[1] != 'B' ||
			    fb->head[2] < '0' || fb->head[2] > '9' ||
			    fb->head[3] < '0' || fb->head[3] > '9' ||
			    (fb->head[2] == '0' && fb->head[3] == '0')) {
				return FB_CLOSE;
			}
			if (fb->ops->send(fb->ops->ctx,
					  (const uint8_t *)"FB01", 4)) {
				return FB_CLOSE;
			}
			fb->phase = PHASE_LENGTH;
			fb->head_len = 0;
			break;
		case PHASE_LENGTH:
			fb->head[fb->head_len++] = *p++;
			n--;
			if (fb->head_len < 8) {
				break;
			}
			fb->msg_len = 0;
			for (int i = 0; i < 8; i++) {
				fb->msg_len = (fb->msg_len << 8) | fb->head[i];
			}
			fb->msg_got = 0;
			if (!fb->downloading && fb->msg_len > FB_COMMAND_MAX) {
				return FB_CLOSE;
			}
			if (fb->msg_len == 0) {
				/* An empty message: nothing to do, and the next
				 * eight bytes are another length. */
				fb->head_len = 0;
				break;
			}
			fb->phase = PHASE_BODY;
			break;
		case PHASE_BODY: {
			size_t took = body(fb, p, n, &result);

			p += took;
			n -= took;
			break;
		}
		}
	}
	return result;
}

int fb_kernel(const uint8_t *img, uint32_t len, uint32_t *offset,
	      uint32_t *size)
{
	/* The version 0 header's fields, little-endian words: the magic
	 * at 0, kernel size at 8 and the page size at 36. */
	if (len < 40 || memcmp(img, "ANDROID!", 8) != 0) {
		*offset = 0;
		*size = len;
		return 0;
	}
	uint32_t ksize = (uint32_t)img[8] | (uint32_t)img[9] << 8 |
			 (uint32_t)img[10] << 16 | (uint32_t)img[11] << 24;
	uint32_t page = (uint32_t)img[36] | (uint32_t)img[37] << 8 |
			(uint32_t)img[38] << 16 | (uint32_t)img[39] << 24;

	if (page == 0 || page > len || ksize > len - page) {
		return 1;
	}
	*offset = page;
	*size = ksize;
	return 0;
}
