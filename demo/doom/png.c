/* SPDX-License-Identifier: Apache-2.0 */
/* See png.h. */
#include "png.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static uint32_t crc_table[256];

static void crc_init(void)
{
	for (uint32_t n = 0; n < 256; n++) {
		uint32_t c = n;
		for (int k = 0; k < 8; k++)
			c = (c & 1) ? 0xedb88320u ^ (c >> 1) : c >> 1;
		crc_table[n] = c;
	}
}

static uint32_t crc(uint32_t c, const uint8_t *p, size_t n)
{
	c ^= 0xffffffffu;
	while (n--)
		c = crc_table[(c ^ *p++) & 0xff] ^ (c >> 8);
	return c ^ 0xffffffffu;
}

static void be32(uint8_t *p, uint32_t v)
{
	p[0] = v >> 24;
	p[1] = v >> 16;
	p[2] = v >> 8;
	p[3] = v;
}

/* One chunk: its length, its type and data, and their CRC. */
static void chunk(FILE *f, const char *type, const uint8_t *data, uint32_t n)
{
	uint8_t head[8];
	be32(head, n);
	memcpy(head + 4, type, 4);
	fwrite(head, 1, 8, f);
	if (n)
		fwrite(data, 1, n, f);
	/* The CRC covers the type and the data together. */
	uint8_t *all = malloc(4 + n);
	memcpy(all, type, 4);
	if (n)
		memcpy(all + 4, data, n);
	be32(head, crc(0, all, 4 + n));
	free(all);
	fwrite(head, 1, 4, f);
}

int png_write(const char *path, const uint32_t *px, int w, int h, int stride)
{
	FILE *f = fopen(path, "wb");
	if (!f)
		return -1;
	crc_init();
	static const uint8_t sig[8] = {137, 'P', 'N', 'G', '\r', '\n', 26, '\n'};
	fwrite(sig, 1, 8, f);
	uint8_t ihdr[13];
	be32(ihdr, w);
	be32(ihdr + 4, h);
	ihdr[8] = 8;  /* bits a channel */
	ihdr[9] = 2;  /* RGB */
	ihdr[10] = 0; /* deflate */
	ihdr[11] = 0; /* the one filter method */
	ihdr[12] = 0; /* not interlaced */
	chunk(f, "IHDR", ihdr, 13);

	/* The raw image: a filter byte of nought, then the row's pixels. */
	size_t row = 1 + 3 * (size_t)w, raw = row * h;
	uint8_t *img = malloc(raw);
	for (int y = 0; y < h; y++) {
		uint8_t *r = img + y * row;
		r[0] = 0;
		for (int x = 0; x < w; x++) {
			uint32_t p = px[y * stride + x];
			r[1 + 3 * x] = p >> 16;
			r[2 + 3 * x] = p >> 8;
			r[3 + 3 * x] = p;
		}
	}
	/* zlib: its header, stored blocks of at most 65535 bytes, Adler-32. */
	size_t blocks = (raw + 65534) / 65535;
	size_t zn = 2 + raw + 5 * blocks + 4;
	uint8_t *z = malloc(zn), *q = z;
	*q++ = 0x78;
	*q++ = 0x01;
	uint32_t a = 1, b = 0;
	for (size_t i = 0; i < raw; i++) {
		a = (a + img[i]) % 65521;
		b = (b + a) % 65521;
	}
	for (size_t at = 0; at < raw; at += 65535) {
		size_t n = raw - at < 65535 ? raw - at : 65535;
		*q++ = at + n == raw;
		*q++ = n & 0xff;
		*q++ = n >> 8;
		*q++ = ~n & 0xff;
		*q++ = (~n >> 8) & 0xff;
		memcpy(q, img + at, n);
		q += n;
	}
	be32(q, b << 16 | a);
	chunk(f, "IDAT", z, zn);
	chunk(f, "IEND", NULL, 0);
	free(img);
	free(z);
	return fclose(f) == 0 ? 0 : -1;
}
