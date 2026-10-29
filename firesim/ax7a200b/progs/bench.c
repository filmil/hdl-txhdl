// SPDX-License-Identifier: Apache-2.0
// A fixed loop, timed by the core's own counters: what one second of
// the host's time buys in target cycles and instructions.
#include "uart.h"
static inline uint64_t cycles(void) { uint64_t v; __asm__ volatile("csrr %0, mcycle" : "=r"(v)); return v; }
static inline uint64_t instret(void) { uint64_t v; __asm__ volatile("csrr %0, minstret" : "=r"(v)); return v; }
int main(void) {
    uart_init();
    puts_("bench: start\n");
    uint64_t c0 = cycles(), i0 = instret();
    volatile uint64_t acc = 0;
    for (uint64_t k = 0; k < 50000000; k++) acc += k ^ (k >> 3);
    uint64_t c1 = cycles(), i1 = instret();
    puts_("bench: cycles "); putdec(c1 - c0);
    puts_(" instret "); putdec(i1 - i0);
    puts_("\n");
    return 0;
}
