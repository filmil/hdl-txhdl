/* The ethslots register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef ETHSLOTS_REGS_H
#define ETHSLOTS_REGS_H

#define ETHSLOTS_SPAN 0x40
#define ETHSLOTS_RX_SLOT 0x00 /* read only: which slot the oldest unacknowledged frame is in */
#define ETHSLOTS_RX_SLOT_SLOT_SHIFT 0 /* read only: the slot */
#define ETHSLOTS_RX_SLOT_SLOT_MASK 0x1
#define ETHSLOTS_RX_SLOT_SLOT_WIDTH 1
#define ETHSLOTS_RX_SLOT_SLOT_RESET 0x0
#define ETHSLOTS_RX_LENGTH 0x04 /* read only: its length in bytes */
#define ETHSLOTS_RX_LENGTH_LENGTH_SHIFT 0 /* read only: the length */
#define ETHSLOTS_RX_LENGTH_LENGTH_MASK 0xffff
#define ETHSLOTS_RX_LENGTH_LENGTH_WIDTH 16
#define ETHSLOTS_RX_LENGTH_LENGTH_RESET 0x0
#define ETHSLOTS_RX_ERRORS 0x08 /* read only: frames dropped because both slots were pending */
#define ETHSLOTS_RX_ERRORS_ERRORS_SHIFT 0 /* read only: the count */
#define ETHSLOTS_RX_ERRORS_ERRORS_MASK 0xffffffff
#define ETHSLOTS_RX_ERRORS_ERRORS_WIDTH 32
#define ETHSLOTS_RX_ERRORS_ERRORS_RESET 0x0
#define ETHSLOTS_RX_EV_STATUS 0x0c /* read only: a frame has arrived and is not acknowledged */
#define ETHSLOTS_RX_EV_STATUS_STATUS_SHIFT 0 /* read only: the event */
#define ETHSLOTS_RX_EV_STATUS_STATUS_MASK 0x1
#define ETHSLOTS_RX_EV_STATUS_STATUS_WIDTH 1
#define ETHSLOTS_RX_EV_STATUS_STATUS_RESET 0x0
#define ETHSLOTS_RX_EV_PENDING 0x10 /* read, write one to clear: a frame arrived and is not acknowledged */
#define ETHSLOTS_RX_EV_PENDING_PENDING_SHIFT 0 /* read, write one to clear: set by an arrival; written one, cleared */
#define ETHSLOTS_RX_EV_PENDING_PENDING_MASK 0x1
#define ETHSLOTS_RX_EV_PENDING_PENDING_WIDTH 1
#define ETHSLOTS_RX_EV_PENDING_PENDING_RESET 0x0
#define ETHSLOTS_RX_EV_ENABLE 0x14 /* read, write: whether an arrival raises the line */
#define ETHSLOTS_RX_EV_ENABLE_ENABLE_SHIFT 0 /* read, write: the enable */
#define ETHSLOTS_RX_EV_ENABLE_ENABLE_MASK 0x1
#define ETHSLOTS_RX_EV_ENABLE_ENABLE_WIDTH 1
#define ETHSLOTS_RX_EV_ENABLE_ENABLE_RESET 0x0
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
#define ETHSLOTS_TX_LEVEL 0x20 /* read only: transmits waiting; one at a time, so zero */
#define ETHSLOTS_TX_LEVEL_LEVEL_SHIFT 0 /* read only: the level */
#define ETHSLOTS_TX_LEVEL_LEVEL_MASK 0x3
#define ETHSLOTS_TX_LEVEL_LEVEL_WIDTH 2
#define ETHSLOTS_TX_LEVEL_LEVEL_RESET 0x0
#define ETHSLOTS_TX_SLOT 0x24 /* write only: which slot the next transmit reads */
#define ETHSLOTS_TX_SLOT_SLOT_SHIFT 0 /* write only: the slot */
#define ETHSLOTS_TX_SLOT_SLOT_MASK 0x1
#define ETHSLOTS_TX_SLOT_SLOT_WIDTH 1
#define ETHSLOTS_TX_SLOT_SLOT_RESET 0x0
#define ETHSLOTS_TX_LENGTH 0x28 /* write only: how many bytes of it to send */
#define ETHSLOTS_TX_LENGTH_LENGTH_SHIFT 0 /* write only: the length */
#define ETHSLOTS_TX_LENGTH_LENGTH_MASK 0xffff
#define ETHSLOTS_TX_LENGTH_LENGTH_WIDTH 16
#define ETHSLOTS_TX_LENGTH_LENGTH_RESET 0x0
#define ETHSLOTS_TX_EV_STATUS 0x2c /* read only: the transmit event; it is never raised */
#define ETHSLOTS_TX_EV_STATUS_STATUS_SHIFT 0 /* read only: the event */
#define ETHSLOTS_TX_EV_STATUS_STATUS_MASK 0x1
#define ETHSLOTS_TX_EV_STATUS_STATUS_WIDTH 1
#define ETHSLOTS_TX_EV_STATUS_STATUS_RESET 0x0
#define ETHSLOTS_TX_EV_PENDING 0x30 /* read, write one to clear: acknowledged by the driver, otherwise unused */
#define ETHSLOTS_TX_EV_PENDING_PENDING_SHIFT 0 /* read, write one to clear: the event */
#define ETHSLOTS_TX_EV_PENDING_PENDING_MASK 0x1
#define ETHSLOTS_TX_EV_PENDING_PENDING_WIDTH 1
#define ETHSLOTS_TX_EV_PENDING_PENDING_RESET 0x0
#define ETHSLOTS_TX_EV_ENABLE 0x34 /* read, write: kept for the driver; it raises nothing */
#define ETHSLOTS_TX_EV_ENABLE_ENABLE_SHIFT 0 /* read, write: the enable */
#define ETHSLOTS_TX_EV_ENABLE_ENABLE_MASK 0x1
#define ETHSLOTS_TX_EV_ENABLE_ENABLE_WIDTH 1
#define ETHSLOTS_TX_EV_ENABLE_ENABLE_RESET 0x0
#define ETHSLOTS_RX_MAC_CHECK 0x38 /* read only: frames the MAC dropped for their check */
#define ETHSLOTS_RX_MAC_CHECK_COUNT_SHIFT 0 /* read only: a failed check sequence or a receive error */
#define ETHSLOTS_RX_MAC_CHECK_COUNT_MASK 0xffff
#define ETHSLOTS_RX_MAC_CHECK_COUNT_WIDTH 16
#define ETHSLOTS_RX_MAC_CHECK_COUNT_RESET 0x0
#define ETHSLOTS_RX_MAC_DROPS 0x3c /* read only: frames the MAC dropped for room or size */
#define ETHSLOTS_RX_MAC_DROPS_ROOM_SHIFT 0 /* read only: both of the MAC's frames were held */
#define ETHSLOTS_RX_MAC_DROPS_ROOM_MASK 0xffff
#define ETHSLOTS_RX_MAC_DROPS_ROOM_WIDTH 16
#define ETHSLOTS_RX_MAC_DROPS_ROOM_RESET 0x0
#define ETHSLOTS_RX_MAC_DROPS_SIZE_SHIFT 16 /* read only: four bytes or fewer, or longer than its store */
#define ETHSLOTS_RX_MAC_DROPS_SIZE_MASK 0xffff0000
#define ETHSLOTS_RX_MAC_DROPS_SIZE_WIDTH 16
#define ETHSLOTS_RX_MAC_DROPS_SIZE_RESET 0x0

#endif
