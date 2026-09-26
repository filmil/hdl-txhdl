/* The eth register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef ETH_REGS_H
#define ETH_REGS_H

#define ETH_SPAN 0x10
#define ETH_STATUS 0x00 /* read only: what the two halves can do */
#define ETH_STATUS_RX_SHIFT 0 /* read only: a received byte is waiting */
#define ETH_STATUS_RX_MASK 0x1
#define ETH_STATUS_RX_WIDTH 1
#define ETH_STATUS_RX_RESET 0x0
#define ETH_STATUS_TX_SHIFT 1 /* read only: the transmitter has room for a byte */
#define ETH_STATUS_TX_MASK 0x2
#define ETH_STATUS_TX_WIDTH 1
#define ETH_STATUS_TX_RESET 0x0
#define ETH_TXBYTE 0x04 /* write only: a byte to send; answered when it is taken */
#define ETH_TXBYTE_DATA_SHIFT 0 /* write only: the byte */
#define ETH_TXBYTE_DATA_MASK 0xff
#define ETH_TXBYTE_DATA_WIDTH 8
#define ETH_TXBYTE_DATA_RESET 0x0
#define ETH_TXBYTE_LAST_SHIFT 8 /* write only: the frame's last byte */
#define ETH_TXBYTE_LAST_MASK 0x100
#define ETH_TXBYTE_LAST_WIDTH 1
#define ETH_TXBYTE_LAST_RESET 0x0
#define ETH_RXBYTE 0x08 /* read; the read takes it: the oldest received byte; a read takes it */
#define ETH_RXBYTE_DATA_SHIFT 0 /* read only: the byte */
#define ETH_RXBYTE_DATA_MASK 0xff
#define ETH_RXBYTE_DATA_WIDTH 8
#define ETH_RXBYTE_DATA_RESET 0x0
#define ETH_RXBYTE_LAST_SHIFT 8 /* read only: the frame's last byte */
#define ETH_RXBYTE_LAST_MASK 0x100
#define ETH_RXBYTE_LAST_WIDTH 1
#define ETH_RXBYTE_LAST_RESET 0x0
#define ETH_RXBYTE_VALID_SHIFT 9 /* read only: a byte was there */
#define ETH_RXBYTE_VALID_MASK 0x200
#define ETH_RXBYTE_VALID_WIDTH 1
#define ETH_RXBYTE_VALID_RESET 0x0

#endif
