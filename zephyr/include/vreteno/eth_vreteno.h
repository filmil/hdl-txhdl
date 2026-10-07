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
	 * length to the stack taking the frame. */
	uint32_t rx_frames;
	uint32_t rx_cycles;
	/* Of those, the copy out of the slot. */
	uint32_t rx_copy_cycles;
	/* Frames sent, the cycles of each send, and of those the copy
	 * into the slot and the read back. */
	uint32_t tx_frames;
	uint32_t tx_cycles;
	uint32_t tx_copy_cycles;
	/* Sends that found the transmitter busy and slept. */
	uint32_t tx_waits;
};

extern struct eth_vreteno_prof eth_vreteno_prof;

#endif
