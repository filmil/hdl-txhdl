// SPDX-License-Identifier: Apache-2.0
// Chipyard's SiFive UART at 0x10020000.
#include <stdint.h>
#define UART ((volatile uint32_t *)0x10020000)
static inline void uart_init(void) { UART[2] = 1; UART[3] = 1; }  // txctrl, rxctrl enable
static inline void putc_(char c) { while ((int32_t)UART[0] < 0) {} UART[0] = (uint8_t)c; }
static inline int getc_(void) { int32_t r = (int32_t)UART[1]; return r < 0 ? -1 : (r & 0xff); }
static inline void puts_(const char *s) { while (*s) putc_(*s++); }
static inline void putdec(uint64_t v) {
    char b[24]; int i = 0;
    do { b[i++] = '0' + v % 10; v /= 10; } while (v);
    while (i) putc_(b[--i]);
}
