/* SPDX-License-Identifier: Apache-2.0 */
/*
 * Fastboot over TCP on the board (issue 143).
 *
 * A TCP server on port 5554, as AOSP's `fastboot/README.md` has the
 * device do, running the core in `../fastboot.c`: the same code the
 * host harness runs against stock `fastboot` in `bazel test`. What
 * this file adds is the board: where a download is staged, and what
 * `boot` does with it.
 *
 *     fastboot -s tcp:192.168.1.50 boot program.bin
 *
 * The download is staged in DDR3 at 0x4800_0000, reserved in this
 * application's overlay. `boot` finds the program in it (the host
 * wraps a plain file in a boot image, see `fb_kernel`), copies it to
 * 0x4000_0000 and jumps there. That is the address the serial loader
 * puts a program at too, so a program built for one loads through
 * the other. The copy overwrites this image, so it runs from the last
 * page of the staging area; see `jump.S`.
 */
#include <zephyr/devicetree.h>
#include <zephyr/irq.h>
#include <zephyr/kernel.h>
#include <zephyr/net/socket.h>
#include <zephyr/sys/printk.h>

#include <errno.h>
#include <string.h>

#include "fastboot.h"

#ifdef CONFIG_ETH_VRETENO_PROFILE
/*
 * Where a download's time goes (issue 1230): the cycles in `recv`,
 * which is waiting and the copy out of the stack, in the fastboot core
 * and of those in the copy into the staging area, the driver's own
 * counts, and every thread's cycles, all across one connection. Said on
 * the console when the connection closes.
 */
#include <vreteno/eth_vreteno.h>

#define PROF_THREADS 16

static struct {
	uint32_t recv_cycles;
	uint32_t recv_calls;
	uint32_t fb_cycles;
	uint32_t write_cycles;
	uint32_t bytes;
	uint32_t start;
	struct eth_vreteno_prof eth;
	const struct k_thread *thread[PROF_THREADS];
	uint64_t cycles[PROF_THREADS];
	int threads;
} prof;

static void prof_thread_start(const struct k_thread *t, void *arg)
{
	k_thread_runtime_stats_t st;

	ARG_UNUSED(arg);
	if (prof.threads == PROF_THREADS ||
	    k_thread_runtime_stats_get((k_tid_t)t, &st) != 0) {
		return;
	}
	prof.thread[prof.threads] = t;
	prof.cycles[prof.threads] = st.execution_cycles;
	prof.threads++;
}

static void prof_begin(void)
{
	memset(&prof, 0, sizeof(prof));
	prof.eth = eth_vreteno_prof;
	k_thread_foreach(prof_thread_start, NULL);
	prof.start = k_cycle_get_32();
}

static void prof_end(void)
{
	uint32_t total = k_cycle_get_32() - prof.start;
	const struct eth_vreteno_prof *e = &eth_vreteno_prof;

	printk("fastboot profile: %u bytes in %u cycles, %u a second\n",
	       prof.bytes, total, (uint32_t)sys_clock_hw_cycles_per_sec());
	printk("  recv %u cycles in %u calls; fastboot %u, of which the "
	       "staging copy %u\n",
	       prof.recv_cycles, prof.recv_calls, prof.fb_cycles,
	       prof.write_cycles);
	printk("  driver rx %u frames, %u cycles, copy %u; tx %u frames, "
	       "%u cycles, copy %u, %u waits\n",
	       e->rx_frames - prof.eth.rx_frames,
	       e->rx_cycles - prof.eth.rx_cycles,
	       e->rx_copy_cycles - prof.eth.rx_copy_cycles,
	       e->tx_frames - prof.eth.tx_frames,
	       e->tx_cycles - prof.eth.tx_cycles,
	       e->tx_copy_cycles - prof.eth.tx_copy_cycles,
	       e->tx_waits - prof.eth.tx_waits);
	for (int i = 0; i < prof.threads; i++) {
		k_thread_runtime_stats_t st;
		const char *name = k_thread_name_get((k_tid_t)prof.thread[i]);

		if (k_thread_runtime_stats_get((k_tid_t)prof.thread[i], &st)) {
			continue;
		}
		printk("  thread %s: %u cycles\n", name ? name : "?",
		       (uint32_t)(st.execution_cycles - prof.cycles[i]));
	}
}
#define PROF_NOW() k_cycle_get_32()
#define PROF_ADD(field, since) (prof.field += k_cycle_get_32() - (since))
#else
#define prof_begin() ((void)0)
#define prof_end() ((void)0)
#define PROF_NOW() 0u
#define PROF_ADD(field, since) ((void)(since))
#endif

#define STAGE_BASE DT_REG_ADDR(DT_NODELABEL(fastboot_stage))
#define STAGE_SIZE DT_REG_SIZE(DT_NODELABEL(fastboot_stage))

/* Where a program runs: the start of DDR3, where this image runs. */
#define RUN_BASE DT_REG_ADDR(DT_NODELABEL(ddr))

/* The last page of the staging area holds the copy routine, so a
 * download stops short of it. */
#define JUMP_PAGE 4096
#define MAX_DOWNLOAD (STAGE_SIZE - JUMP_PAGE)

/*
 * The longest program `boot` will run. The Ethernet engines store
 * received frames at 0x4100_0000, 16 MiB above where a program goes,
 * and they keep doing so after the jump, so a program that reached
 * past there could have a frame land in the middle of it.
 */
#define MAX_PROGRAM 0x01000000u

#define PORT 5554

extern const uint8_t fb_jump[];
extern const uint8_t fb_jump_end[];

struct board {
	int fd;
};

static int board_write(void *ctx, uint32_t offset, const uint8_t *p,
		       uint32_t n)
{
	ARG_UNUSED(ctx);
	if (offset > MAX_DOWNLOAD || n > MAX_DOWNLOAD - offset) {
		return 1;
	}
	uint32_t t0 = PROF_NOW();

	memcpy((uint8_t *)STAGE_BASE + offset, p, n);
	PROF_ADD(write_cycles, t0);
	return 0;
}

static int board_send(void *ctx, const uint8_t *p, size_t n)
{
	struct board *b = ctx;

	while (n > 0) {
		ssize_t k = zsock_send(b->fd, p, n, 0);

		if (k <= 0) {
			return 1;
		}
		p += k;
		n -= (size_t)k;
	}
	return 0;
}

static int board_getvar(void *ctx, const char *name, char *out,
			size_t size)
{
	ARG_UNUSED(ctx);
	if (strcmp(name, "product") == 0) {
		strncpy(out, "vreteno", size);
		return 0;
	}
	return 1;
}

/* Run what was staged; returns only if it cannot. */
static void boot(uint32_t staged)
{
	const uint8_t *img = (const uint8_t *)STAGE_BASE;
	uint32_t off, size;

	if (fb_kernel(img, staged, &off, &size) || size == 0 ||
	    size > MAX_PROGRAM) {
		printk("fastboot: the image holds no program that fits\n");
		return;
	}
	printk("fastboot: booting %u bytes at 0x%08x\n", size,
	       (unsigned int)RUN_BASE);

	size_t jump_len = fb_jump_end - fb_jump;
	uint8_t *jump = (uint8_t *)(STAGE_BASE + STAGE_SIZE - JUMP_PAGE);

	/* No interrupt from here on: the handlers are in the image the
	 * copy is about to overwrite. */
	(void)irq_lock();
	memcpy(jump, fb_jump, jump_len);
	/* The routine's own words must have landed before it is fetched,
	 * for the reason `jump.S` gives for the program's. */
	for (size_t i = 0; i < 4 && 4 * (i + 1) <= jump_len; i++) {
		(void)*(volatile const uint32_t *)(jump + jump_len - 4 * (i + 1));
	}

	void (*run)(uint32_t, uint32_t, uint32_t) =
		(void (*)(uint32_t, uint32_t, uint32_t))(uintptr_t)jump;

	run(STAGE_BASE + off, RUN_BASE, (size + 3) & ~3u);
}

int main(void)
{
	static struct board b;
	static struct fb fb;
	static const struct fb_ops ops = {
		.max_download = MAX_DOWNLOAD,
		.write = board_write,
		.send = board_send,
		.getvar = board_getvar,
		.ctx = &b,
	};
	struct sockaddr_in addr = {
		.sin_family = AF_INET,
		.sin_port = htons(PORT),
		.sin_addr.s_addr = htonl(INADDR_ANY),
	};
	int ls = zsock_socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);

	if (ls < 0 ||
	    zsock_bind(ls, (struct sockaddr *)&addr, sizeof(addr)) < 0 ||
	    zsock_listen(ls, 1) < 0) {
		printk("fastboot: cannot listen on %d: %d\n", PORT, errno);
		return 0;
	}
	printk("fastboot: listening on port %d\n", PORT);

	/* A download outlives its connection, as in the host harness. */
	uint32_t staged = 0;

	for (;;) {
		static uint8_t buf[1500];
		enum fb_result r = FB_MORE;

		b.fd = zsock_accept(ls, NULL, NULL);
		if (b.fd < 0) {
			continue;
		}
		fb_init(&fb, &ops);
		fb.staged = staged;
		prof_begin();
		while (r == FB_MORE) {
			uint32_t t0 = PROF_NOW();
			ssize_t k = zsock_recv(b.fd, buf, sizeof(buf), 0);

			PROF_ADD(recv_cycles, t0);
#ifdef CONFIG_ETH_VRETENO_PROFILE
			prof.recv_calls++;
#endif
			if (k <= 0) {
				break;
			}
#ifdef CONFIG_ETH_VRETENO_PROFILE
			prof.bytes += (uint32_t)k;
#endif
			t0 = PROF_NOW();
			r = fb_input(&fb, buf, (size_t)k);
			PROF_ADD(fb_cycles, t0);
		}
		prof_end();
		staged = fb.staged;
		/* Close before acting, so the host has its `OKAY`. */
		zsock_close(b.fd);
		if (r == FB_BOOT) {
			k_msleep(100);
			boot(staged);
		}
	}
	return 0;
}
