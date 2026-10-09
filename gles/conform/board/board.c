/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The conformance suite on the board (#999): EGL's machine the board's,
 * from the GL library built for the core, and newlib's system calls,
 * standard output to the serial port, the heap the DDR3 between the
 * program and Razboj's framebuffer.
 */
#include <errno.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <sys/stat.h>

#include "uart.h"

#undef errno
extern int errno;

/* The board's machine for EGL, from the library (gles/vreteno/zephyr.rs). */
extern void egl_vreteno_install(void);

void conform_machine(void) { egl_vreteno_install(); }

#define UART_AT 0x00003000u

static uint32_t rd(uint32_t at) { return *(volatile uint32_t *)at; }
static void wr(uint32_t at, uint32_t v) { *(volatile uint32_t *)at = v; }

static void uart_put(char c) {
  while (rd(UART_AT + UART_TXDATA) & UART_TXDATA_FULL_MASK) {
  }
  wr(UART_AT + UART_TXDATA, (unsigned char)c);
}

static void uart_hex(uint32_t v) {
  for (int s = 28; s >= 0; s -= 4)
    uart_put("0123456789abcdef"[(v >> s) & 15]);
}

static void uart_say(const char *s) {
  while (*s)
    uart_put(*s++);
}

/* What `start.S`'s trap says (#1536), a line of its own, written to the
 * serial port directly rather than through the C library, which the
 * trap may have come from. */
void conform_trap(uint32_t cause, uint32_t pc, uint32_t value) {
  uart_say("\r\nTRAP mcause 0x");
  uart_hex(cause);
  uart_say(" mepc 0x");
  uart_hex(pc);
  uart_say(" mtval 0x");
  uart_hex(value);
  uart_say("\r\n");
}

extern char __heap_start[], __heap_end[];
static char *brk = __heap_start;

void *_sbrk(ptrdiff_t n) {
  if (brk + n > __heap_end || brk + n < __heap_start) {
    errno = ENOMEM;
    return (void *)-1;
  }
  char *was = brk;
  brk += n;
  return was;
}

int _write(int fd, const void *buf, size_t n) {
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
  return (int)n;
}

int _read(int fd, void *buf, size_t n) {
  (void)fd, (void)buf, (void)n;
  return 0;
}
int _close(int fd) {
  (void)fd;
  return -1;
}
int _lseek(int fd, int off, int whence) {
  (void)fd, (void)off, (void)whence;
  return 0;
}
int _fstat(int fd, struct stat *st) {
  (void)fd;
  memset(st, 0, sizeof *st);
  st->st_mode = S_IFCHR;
  return 0;
}
int _isatty(int fd) { return fd < 3; }
int _getpid(void) { return 1; }
int _kill(int pid, int sig) {
  (void)pid, (void)sig;
  errno = EINVAL;
  return -1;
}
void _exit(int code) {
  (void)code;
  __asm__ volatile("csrwi 0x7c0, 1");
  for (;;) {
  }
}
