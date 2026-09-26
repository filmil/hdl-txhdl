/* The uart register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef UART_REGS_H
#define UART_REGS_H

#define UART_SPAN 0x10
#define UART_TX 0x00 /* read, write: written, a byte to send; read, the last byte sent */
#define UART_TX_DATA_SHIFT 0 /* read, write: the byte */
#define UART_TX_DATA_MASK 0xff
#define UART_TX_DATA_WIDTH 8
#define UART_TX_DATA_RESET 0x0
#define UART_STATUS 0x04 /* read only: what the port is doing */
#define UART_STATUS_BUSY_SHIFT 0 /* read only: a byte is going out */
#define UART_STATUS_BUSY_MASK 0x1
#define UART_STATUS_BUSY_WIDTH 1
#define UART_STATUS_BUSY_RESET 0x0
#define UART_STATUS_READY_SHIFT 1 /* read only: a byte received and not yet read */
#define UART_STATUS_READY_MASK 0x2
#define UART_STATUS_READY_WIDTH 1
#define UART_STATUS_READY_RESET 0x0
#define UART_STATUS_FULL_SHIFT 2 /* read only: the buffer of eight is full */
#define UART_STATUS_FULL_MASK 0x4
#define UART_STATUS_FULL_WIDTH 1
#define UART_STATUS_FULL_RESET 0x0
#define UART_RX 0x08 /* read; the read takes it: the oldest byte received; the read takes it */
#define UART_RX_DATA_SHIFT 0 /* read; the read takes it: the byte */
#define UART_RX_DATA_MASK 0xff
#define UART_RX_DATA_WIDTH 8
#define UART_RX_DATA_RESET 0x0

#endif
