/* The ethslots register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef ETHSLOTS_REGS_H
#define ETHSLOTS_REGS_H

#define ETHSLOTS_SPAN 0x40
#define ETHSLOTS_RX_SLOT 0x00 /* read only: which slot the last frame is in */
#define ETHSLOTS_RX_SLOT_SLOT_SHIFT 0 /* read only: the slot */
#define ETHSLOTS_RX_SLOT_SLOT_MASK 0x1
#define ETHSLOTS_RX_SLOT_SLOT_WIDTH 1
#define ETHSLOTS_RX_SLOT_SLOT_RESET 0x0
#define ETHSLOTS_RX_LENGTH 0x04 /* read only: its length in bytes */
#define ETHSLOTS_RX_LENGTH_LENGTH_SHIFT 0 /* read only: the length */
#define ETHSLOTS_RX_LENGTH_LENGTH_MASK 0xffff
#define ETHSLOTS_RX_LENGTH_LENGTH_WIDTH 16
#define ETHSLOTS_RX_LENGTH_LENGTH_RESET 0x0
#define ETHSLOTS_RX_EV_PENDING 0x08 /* read, write one to clear: a frame arrived and is not acknowledged */
#define ETHSLOTS_RX_EV_PENDING_PENDING_SHIFT 0 /* read, write one to clear: set by an arrival; written one, cleared */
#define ETHSLOTS_RX_EV_PENDING_PENDING_MASK 0x1
#define ETHSLOTS_RX_EV_PENDING_PENDING_WIDTH 1
#define ETHSLOTS_RX_EV_PENDING_PENDING_RESET 0x0
#define ETHSLOTS_RX_EV_ENABLE 0x0c /* read, write: whether an arrival raises the line */
#define ETHSLOTS_RX_EV_ENABLE_ENABLE_SHIFT 0 /* read, write: the enable */
#define ETHSLOTS_RX_EV_ENABLE_ENABLE_MASK 0x1
#define ETHSLOTS_RX_EV_ENABLE_ENABLE_WIDTH 1
#define ETHSLOTS_RX_EV_ENABLE_ENABLE_RESET 0x0
#define ETHSLOTS_TX_SLOT 0x10 /* write only: which slot the next transmit reads */
#define ETHSLOTS_TX_SLOT_SLOT_SHIFT 0 /* write only: the slot */
#define ETHSLOTS_TX_SLOT_SLOT_MASK 0x1
#define ETHSLOTS_TX_SLOT_SLOT_WIDTH 1
#define ETHSLOTS_TX_SLOT_SLOT_RESET 0x0
#define ETHSLOTS_TX_LENGTH 0x14 /* write only: how many bytes of it to send */
#define ETHSLOTS_TX_LENGTH_LENGTH_SHIFT 0 /* write only: the length */
#define ETHSLOTS_TX_LENGTH_LENGTH_MASK 0xffff
#define ETHSLOTS_TX_LENGTH_LENGTH_WIDTH 16
#define ETHSLOTS_TX_LENGTH_LENGTH_RESET 0x0
#define ETHSLOTS_TX_START 0x18 /* write only: written one, the transmit starts */
#define ETHSLOTS_TX_START_START_SHIFT 0 /* write only: the start */
#define ETHSLOTS_TX_START_START_MASK 0x1
#define ETHSLOTS_TX_START_START_WIDTH 1
#define ETHSLOTS_TX_START_START_RESET 0x0
#define ETHSLOTS_TX_READY 0x1c /* read only: no transmit is running or waiting */
#define ETHSLOTS_TX_READY_READY_SHIFT 0 /* read only: ready */
#define ETHSLOTS_TX_READY_READY_MASK 0x1
#define ETHSLOTS_TX_READY_READY_WIDTH 1
#define ETHSLOTS_TX_READY_READY_RESET 0x1
#define ETHSLOTS_TX_EV_PENDING 0x20 /* read, write one to clear: acknowledged by the driver, otherwise unused */
#define ETHSLOTS_TX_EV_PENDING_PENDING_SHIFT 0 /* read, write one to clear: the event */
#define ETHSLOTS_TX_EV_PENDING_PENDING_MASK 0x1
#define ETHSLOTS_TX_EV_PENDING_PENDING_WIDTH 1
#define ETHSLOTS_TX_EV_PENDING_PENDING_RESET 0x0
#define ETHSLOTS_TX_EV_ENABLE 0x24 /* read, write: kept for the driver; it raises nothing */
#define ETHSLOTS_TX_EV_ENABLE_ENABLE_SHIFT 0 /* read, write: the enable */
#define ETHSLOTS_TX_EV_ENABLE_ENABLE_MASK 0x1
#define ETHSLOTS_TX_EV_ENABLE_ENABLE_WIDTH 1
#define ETHSLOTS_TX_EV_ENABLE_ENABLE_RESET 0x0

#endif
