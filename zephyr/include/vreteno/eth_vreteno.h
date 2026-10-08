/* SPDX-License-Identifier: Apache-2.0 */
/*
 * What the Vreteno Ethernet driver counts with
 * `CONFIG_ETH_VRETENO_PROFILE` (issue 1230): frames each way, and the
 * core's cycles in each path, as `k_cycle_get_32` counts them. The
 * counts only grow; a reader takes the difference across what it
 * measures.
 */
#ifndef VRETENO_ETH_VRETENO_H
#define VRETENO_ETH_VRETENO_H

#include <stdint.h>

struct eth_vreteno_prof {
	/* Frames handed to the stack, and the cycles from reading the
	 * length to the stack taking the frame. The cycles are 64 bits:
	 * a download of a minute is six billion of them, past 32 bits
	 * (issue 1377). */
	uint32_t rx_frames;
	uint64_t rx_cycles;
	/* Of those, the copy out of the slot. */
	uint64_t rx_copy_cycles;
	/* Frames the driver could not hand on: no buffer for them, or the
	 * stack would not take them. Each is a frame lost. */
	uint32_t rx_nobuf;
	uint32_t rx_refused;
	/* Frames sent, the cycles of each send, and of those the copy
	 * into the slot and the read back. */
	uint32_t tx_frames;
	uint64_t tx_cycles;
	uint64_t tx_copy_cycles;
	/* Sends that found the transmitter busy and slept. */
	uint32_t tx_waits;
};

extern struct eth_vreteno_prof eth_vreteno_prof;

/* The port's `rx_errors`: the frames it dropped because both receive
 * slots held one the driver had not acknowledged (issue 1313). */
uint32_t eth_vreteno_rx_errors(void);

/* The frames the MAC dropped ahead of the slots, by cause, each a
 * sixteen-bit count that wraps (issue 1404): no room, both of its two
 * frames held; a failed check sequence or a receive error; and a
 * frame of four bytes or fewer, or longer than its store. */
struct eth_vreteno_mac_drops {
	uint16_t room;
	uint16_t check;
	uint16_t size;
};

void eth_vreteno_mac_drops(struct eth_vreteno_mac_drops *d);

#endif
