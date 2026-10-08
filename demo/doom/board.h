/* SPDX-License-Identifier: Apache-2.0 */
/* The board as Doom's port sees it (issue 1177). */
#ifndef DOOM_BOARD_H
#define DOOM_BOARD_H

#include <stdint.h>

/* The core's clock. */
#define BOARD_HZ 100000000u

/* The cycle counter, all 64 bits. */
uint64_t board_cycles(void);

/* A byte out of the serial port, waiting while its queue is full; and a
 * byte in, or -1 when none has arrived. */
void uart_put(char c);
int uart_get(void);

#endif
