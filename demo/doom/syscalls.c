/* SPDX-License-Identifier: Apache-2.0 */
/*
 * newlib's system calls for Doom on the board (issue 1177).
 *
 * The heap is the DDR3 between the program's data and Razboj's
 * framebuffer; standard output and error go to the serial port; and one
 * file can be opened, the WAD, which is memory: the ramdisk of the boot
 * image fastboot staged at 0x4800_0000, or, where nothing staged one, a
 * WAD laid at that address bare, as the machine model's `--dtb` lays a
 * file. Every other file is not there, so the game's configuration and
 * saved games are refused and it goes on without them.
 */
#include <errno.h>
#include <stdint.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/times.h>

#include "board.h"

#undef errno
extern int errno;

extern char __heap_start[], __heap_end[];
static char *brk = __heap_start;

void *_sbrk(ptrdiff_t n)
{
	if (brk + n > __heap_end || brk + n < __heap_start) {
		errno = ENOMEM;
		return (void *)-1;
	}
	char *was = brk;
	brk += n;
	return was;
}

/* Where fastboot stages a download: `fastboot_stage` in its overlay. */
#define STAGE 0x48000000u

static const uint8_t *wad_at;
static uint32_t wad_size;
static uint32_t wad_pos;
static int wad_open;

static uint32_t le32(const uint8_t *p)
{
	return p[0] | p[1] << 8 | p[2] << 16 | (uint32_t)p[3] << 24;
}

/* The WAD: the staged boot image's ramdisk, the version 0 header's
 * kernel size at 8, ramdisk size at 16 and page size at 36, the ramdisk
 * starting at the first page after the kernel's; or a WAD at the stage
 * itself. */
static int wad_find(void)
{
	const uint8_t *s = (const uint8_t *)STAGE;
	if (memcmp(s, "ANDROID!", 8) == 0) {
		uint32_t kernel = le32(s + 8), ramdisk = le32(s + 16);
		uint32_t page = le32(s + 36);
		if (page == 0 || ramdisk == 0)
			return -1;
		uint32_t kpages = (kernel + page - 1) / page;
		wad_at = s + page * (1 + kpages);
		wad_size = ramdisk;
	} else if (memcmp(s, "IWAD", 4) == 0 || memcmp(s, "PWAD", 4) == 0) {
		wad_at = s;
		/* The directory's end is the WAD's: its count at 4 and its
		 * offset at 8, sixteen bytes an entry. */
		wad_size = le32(s + 8) + 16 * le32(s + 4);
	} else {
		return -1;
	}
	return memcmp(wad_at, "IWAD", 4) == 0 || memcmp(wad_at, "PWAD", 4) == 0
		       ? 0
		       : -1;
}

/* The WAD's name, as `main` passes it to the game. */
#define WAD_NAME "doom.wad"
#define WAD_FD 3

int _open(const char *path, int flags, int mode)
{
	(void)mode;
	const char *base = strrchr(path, '/');
	base = base ? base + 1 : path;
	if (strcmp(base, WAD_NAME) != 0 || (flags & 3) != 0) {
		errno = ENOENT;
		return -1;
	}
	if (!wad_at && wad_find() != 0) {
		errno = ENOENT;
		return -1;
	}
	wad_open = 1;
	wad_pos = 0;
	return WAD_FD;
}

int _close(int fd)
{
	if (fd == WAD_FD)
		wad_open = 0;
	return 0;
}

int _read(int fd, void *buf, size_t n)
{
	if (fd != WAD_FD || !wad_open)
		return 0;
	if (wad_pos >= wad_size)
		return 0;
	if (n > wad_size - wad_pos)
		n = wad_size - wad_pos;
	memcpy(buf, wad_at + wad_pos, n);
	wad_pos += n;
	return n;
}

int _lseek(int fd, int off, int whence)
{
	if (fd != WAD_FD) {
		errno = ESPIPE;
		return -1;
	}
	int64_t at = whence == 0 ? off : whence == 1 ? (int64_t)wad_pos + off
						    : (int64_t)wad_size + off;
	if (at < 0) {
		errno = EINVAL;
		return -1;
	}
	wad_pos = at;
	return wad_pos;
}

int _write(int fd, const void *buf, size_t n)
{
	if (fd != 1 && fd != 2) {
		errno = EBADF;
		return -1;
	}
	const char *c = buf;
	for (size_t i = 0; i < n; i++) {
		if (c[i] == '\n')
			uart_put('\r');
		uart_put(c[i]);
	}
	return n;
}

int _fstat(int fd, struct stat *st)
{
	memset(st, 0, sizeof *st);
	if (fd == WAD_FD) {
		st->st_mode = S_IFREG;
		st->st_size = wad_size;
	} else {
		st->st_mode = S_IFCHR;
	}
	return 0;
}

int _stat(const char *path, struct stat *st)
{
	int fd = _open(path, 0, 0);
	if (fd < 0)
		return -1;
	_fstat(fd, st);
	_close(fd);
	return 0;
}

int _isatty(int fd) { return fd < 3; }

int _getpid(void) { return 1; }

int _kill(int pid, int sig)
{
	(void)pid;
	(void)sig;
	errno = EINVAL;
	return -1;
}

void _exit(int code)
{
	(void)code;
	__asm__ volatile("csrwi 0x7c0, 1");
	for (;;) {
	}
}

int _unlink(const char *path)
{
	(void)path;
	errno = ENOENT;
	return -1;
}

int _link(const char *a, const char *b)
{
	(void)a;
	(void)b;
	errno = EMLINK;
	return -1;
}

int _rename(const char *a, const char *b)
{
	(void)a;
	(void)b;
	errno = ENOENT;
	return -1;
}

int mkdir(const char *path, mode_t mode)
{
	(void)path;
	(void)mode;
	errno = EROFS;
	return -1;
}

int _gettimeofday(struct timeval *tv, void *tz)
{
	(void)tz;
	uint64_t us = board_cycles() / (BOARD_HZ / 1000000u);
	tv->tv_sec = us / 1000000u;
	tv->tv_usec = us % 1000000u;
	return 0;
}

clock_t _times(struct tms *t)
{
	clock_t c = board_cycles() / (BOARD_HZ / 1000u);
	t->tms_utime = c;
	t->tms_stime = t->tms_cutime = t->tms_cstime = 0;
	return c;
}
