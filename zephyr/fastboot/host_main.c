/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The fastboot core on a host socket, so stock `fastboot` can be run
 * against the code the board runs, with no board (issue 143).
 *
 *     fastboot_host PORT_FILE OUT_FILE
 *
 * Listens on a free port of 127.0.0.1 and writes its number to
 * PORT_FILE, so a test can start the tool against it without choosing
 * a port that might be taken. Serves one connection after another, as
 * the tool opens one per invocation. On `boot` it finds the program in
 * what was staged, the way the board does, writes those bytes to
 * OUT_FILE and exits 0; on `reboot` it exits 0 having written nothing.
 */
#include "fastboot.h"

#include <arpa/inet.h>
#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

/* What `max-download-size` says, and what is set aside for it. */
#define STAGE_BYTES (16u << 20)

struct host {
	int fd;
	uint8_t *stage;
};

static int host_write(void *ctx, uint32_t offset, const uint8_t *p,
		      uint32_t n)
{
	struct host *h = ctx;

	if (offset > STAGE_BYTES || n > STAGE_BYTES - offset) {
		return 1;
	}
	memcpy(h->stage + offset, p, n);
	return 0;
}

static int host_send(void *ctx, const uint8_t *p, size_t n)
{
	struct host *h = ctx;

	while (n > 0) {
		ssize_t k = send(h->fd, p, n, 0);

		if (k <= 0) {
			return 1;
		}
		p += k;
		n -= (size_t)k;
	}
	return 0;
}

static int host_getvar(void *ctx, const char *name, char *out,
		       size_t size)
{
	(void)ctx;
	if (strcmp(name, "product") == 0) {
		snprintf(out, size, "vreteno");
		return 0;
	}
	return 1;
}

int main(int argc, char **argv)
{
	if (argc != 3) {
		fprintf(stderr, "usage: %s PORT_FILE OUT_FILE\n", argv[0]);
		return 2;
	}
	struct host h = {.fd = -1, .stage = malloc(STAGE_BYTES)};
	struct fb_ops ops = {
		.max_download = STAGE_BYTES,
		.write = host_write,
		.send = host_send,
		.can_reboot = 1,
		.getvar = host_getvar,
		.ctx = &h,
	};
	int ls = socket(AF_INET, SOCK_STREAM, 0);
	struct sockaddr_in addr = {
		.sin_family = AF_INET,
		.sin_addr.s_addr = htonl(INADDR_LOOPBACK),
	};
	socklen_t alen = sizeof(addr);

	if (!h.stage || ls < 0 ||
	    bind(ls, (struct sockaddr *)&addr, sizeof(addr)) ||
	    listen(ls, 1) ||
	    getsockname(ls, (struct sockaddr *)&addr, &alen)) {
		perror("fastboot_host");
		return 1;
	}
	FILE *pf = fopen(argv[1], "w");

	if (!pf) {
		perror(argv[1]);
		return 1;
	}
	fprintf(pf, "%u\n", ntohs(addr.sin_port));
	fclose(pf);

	/* Staged bytes outlive a connection: `fastboot boot` downloads
	 * and boots on one, but nothing says it must. */
	uint32_t staged = 0;

	for (;;) {
		struct fb fb;

		h.fd = accept(ls, NULL, NULL);
		if (h.fd < 0) {
			perror("accept");
			return 1;
		}
		fb_init(&fb, &ops);
		fb.staged = staged;
		enum fb_result r = FB_MORE;
		uint8_t buf[1500];

		while (r == FB_MORE) {
			ssize_t k = recv(h.fd, buf, sizeof(buf), 0);

			if (k <= 0) {
				break;
			}
			r = fb_input(&fb, buf, (size_t)k);
		}
		staged = fb.staged;
		close(h.fd);
		if (r == FB_REBOOT) {
			return 0;
		}
		if (r == FB_BOOT) {
			uint32_t off, size;

			if (fb_kernel(h.stage, staged, &off, &size)) {
				fprintf(stderr, "fastboot_host: bad image\n");
				return 1;
			}
			FILE *out = fopen(argv[2], "wb");

			if (!out || fwrite(h.stage + off, 1, size, out) != size) {
				perror(argv[2]);
				return 1;
			}
			fclose(out);
			return 0;
		}
	}
}
