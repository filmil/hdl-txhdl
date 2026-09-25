/* SPDX-License-Identifier: Apache-2.0 */
/*
 * Fastboot over TCP, as a device speaks it, with no transport in it.
 *
 * The protocol is AOSP's, from `fastboot/README.md`: the host opens a
 * TCP connection to the device on port 5554, both sides send `FB01`,
 * and from then on every message in either direction is an eight byte
 * big-endian length followed by that many bytes. A command is ASCII of
 * at most 4096 bytes; an answer is `OKAY`, `FAIL`, `DATA` or `INFO`
 * and a message (issue 143).
 *
 * This file knows nothing of sockets. Bytes come in through
 * `fb_input`, in whatever pieces the transport happens to deliver,
 * and answers go out through `fb_ops.send`. The same code is what the
 * Zephyr server runs on the board and what the host harness runs
 * against stock `fastboot`, so the test that the tool and this code
 * agree is a test of the code the board runs.
 *
 * What it does: `getvar`, `download` and `boot`, which is what
 * `fastboot boot <image>` needs, and `reboot` where the platform can
 * reset. It refuses `flash`, `erase` and anything else with `FAIL`,
 * since nothing on this machine writes persistent storage.
 */
#ifndef TXHDL_FASTBOOT_H
#define TXHDL_FASTBOOT_H

#include <stddef.h>
#include <stdint.h>

/* The longest command the protocol allows. */
#define FB_COMMAND_MAX 4096

/* What `fb_input` asks of its caller, after it has sent any answer. */
enum fb_result {
	/* Keep reading. */
	FB_MORE = 0,
	/* Close the connection: the peer is not speaking fastboot. */
	FB_CLOSE,
	/* `boot` was answered `OKAY`: run what was downloaded. */
	FB_BOOT,
	/* `reboot` was answered `OKAY`: reset the machine. */
	FB_REBOOT,
};

struct fb_ops {
	/* The most `download` takes, in bytes. */
	uint32_t max_download;
	/* Store `n` downloaded bytes at `offset` into the image. Returns
	 * zero, or nonzero to fail the download. */
	int (*write)(void *ctx, uint32_t offset, const uint8_t *p,
		     uint32_t n);
	/* Send `n` bytes to the host. Returns zero on success. */
	int (*send)(void *ctx, const uint8_t *p, size_t n);
	/* Nonzero if the platform can act on `reboot`; without it the core
	 * answers `FAIL` rather than promise a reset that will not come. */
	int can_reboot;
	/* The value of a variable the core does not know itself, into
	 * `out` of `size` bytes; nonzero if there is none. May be NULL. */
	int (*getvar)(void *ctx, const char *name, char *out, size_t size);
	void *ctx;
};

struct fb {
	const struct fb_ops *ops;
	/* Where the stream is: the handshake, a message's length, or a
	 * message's bytes. */
	int phase;
	/* The bytes of the handshake or of a length seen so far. */
	uint8_t head[8];
	size_t head_len;
	/* The message being read: its length and how much has come. */
	uint64_t msg_len;
	uint64_t msg_got;
	/* A command being gathered, terminated when it is whole. */
	char cmd[FB_COMMAND_MAX + 1];
	/* Download state: how much was asked for and how much has come.
	 * `downloading` is set between `DATA` and the last byte. */
	int downloading;
	uint32_t want;
	uint32_t got;
	/* The size of the last complete download, zero for none. */
	uint32_t staged;
};

void fb_init(struct fb *fb, const struct fb_ops *ops);

/* Hand the core `n` bytes the transport received. The answer to any
 * command they complete is sent before this returns. */
enum fb_result fb_input(struct fb *fb, const uint8_t *p, size_t n);

/*
 * Where the program is in a staged image. `fastboot boot <file>` wraps
 * a file that is not already an Android boot image in a version 0
 * header, the magic `ANDROID!` with the kernel at the header's page
 * size; a staged image without that magic is the program itself. Sets
 * `offset` and `size` and returns zero, or returns nonzero for a
 * header that says more than the image holds.
 */
int fb_kernel(const uint8_t *img, uint32_t len, uint32_t *offset,
	      uint32_t *size);

#endif
