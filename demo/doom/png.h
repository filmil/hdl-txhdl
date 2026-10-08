/* SPDX-License-Identifier: Apache-2.0 */
/*
 * A PNG of 32-bit pixels, 0x00RRGGBB, written with no compression
 * (issue 1177): the image data as zlib's stored blocks, which every
 * PNG reader takes, so the writer needs no zlib.
 */
#ifndef DOOM_PNG_H
#define DOOM_PNG_H

#include <stdint.h>

/* Writes `w` by `h` pixels, `stride` words a row, to `path`. Returns 0, or
 * -1 if the file cannot be written. */
int png_write(const char *path, const uint32_t *px, int w, int h,
	      int stride);

#endif
