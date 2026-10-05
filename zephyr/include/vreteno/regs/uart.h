/* The uart register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef UART_REGS_H
#define UART_REGS_H

#define UART_SPAN 0x20
#define UART_TXDATA 0x00 /* read, write: a byte to send, and whether the queue is full */
#define UART_TXDATA_DATA_SHIFT 0 /* write only: the byte to send */
#define UART_TXDATA_DATA_MASK 0xff
#define UART_TXDATA_DATA_WIDTH 8
#define UART_TXDATA_DATA_RESET 0x0
#define UART_TXDATA_FULL_SHIFT 31 /* read only: the queue is full; a byte written now is dropped */
#define UART_TXDATA_FULL_MASK 0x80000000
#define UART_TXDATA_FULL_WIDTH 1
#define UART_TXDATA_FULL_RESET 0x0
#define UART_RXDATA 0x04 /* read; the read takes it: the oldest byte received */
#define UART_RXDATA_DATA_SHIFT 0 /* read only: the byte */
#define UART_RXDATA_DATA_MASK 0xff
#define UART_RXDATA_DATA_WIDTH 8
#define UART_RXDATA_DATA_RESET 0x0
#define UART_RXDATA_EMPTY_SHIFT 31 /* read only: nothing was waiting, and the byte is not one */
#define UART_RXDATA_EMPTY_MASK 0x80000000
#define UART_RXDATA_EMPTY_WIDTH 1
#define UART_RXDATA_EMPTY_RESET 0x1
#define UART_TXCTRL 0x08 /* read, write: the transmitter's controls */
#define UART_TXCTRL_TXEN_SHIFT 0 /* read, write: send what the queue holds */
#define UART_TXCTRL_TXEN_MASK 0x1
#define UART_TXCTRL_TXEN_WIDTH 1
#define UART_TXCTRL_TXEN_RESET 0x1
#define UART_TXCTRL_NSTOP_SHIFT 1 /* read, write: two stop bits rather than one */
#define UART_TXCTRL_NSTOP_MASK 0x2
#define UART_TXCTRL_NSTOP_WIDTH 1
#define UART_TXCTRL_NSTOP_RESET 0x0
#define UART_TXCTRL_TXCNT_SHIFT 16 /* read, write: txwm is pending while fewer than this wait */
#define UART_TXCTRL_TXCNT_MASK 0x70000
#define UART_TXCTRL_TXCNT_WIDTH 3
#define UART_TXCTRL_TXCNT_RESET 0x0
#define UART_RXCTRL 0x0c /* read, write: the receiver's controls */
#define UART_RXCTRL_RXEN_SHIFT 0 /* read, write: take frames from the line */
#define UART_RXCTRL_RXEN_MASK 0x1
#define UART_RXCTRL_RXEN_WIDTH 1
#define UART_RXCTRL_RXEN_RESET 0x1
#define UART_RXCTRL_RXCNT_SHIFT 16 /* read, write: rxwm is pending while more than this wait */
#define UART_RXCTRL_RXCNT_MASK 0x70000
#define UART_RXCTRL_RXCNT_WIDTH 3
#define UART_RXCTRL_RXCNT_RESET 0x0
#define UART_IE 0x10 /* read, write: which watermarks raise the interrupt line */
#define UART_IE_TXWM_SHIFT 0 /* read, write: the transmit watermark */
#define UART_IE_TXWM_MASK 0x1
#define UART_IE_TXWM_WIDTH 1
#define UART_IE_TXWM_RESET 0x0
#define UART_IE_RXWM_SHIFT 1 /* read, write: the receive watermark */
#define UART_IE_RXWM_MASK 0x2
#define UART_IE_RXWM_WIDTH 1
#define UART_IE_RXWM_RESET 0x0
#define UART_IP 0x14 /* read only: which watermarks are passed */
#define UART_IP_TXWM_SHIFT 0 /* read only: fewer than txcnt bytes wait to be sent */
#define UART_IP_TXWM_MASK 0x1
#define UART_IP_TXWM_WIDTH 1
#define UART_IP_TXWM_RESET 0x0
#define UART_IP_RXWM_SHIFT 1 /* read only: more than rxcnt bytes wait to be read */
#define UART_IP_RXWM_MASK 0x2
#define UART_IP_RXWM_WIDTH 1
#define UART_IP_RXWM_RESET 0x0
#define UART_DIV 0x18 /* read, write: the bit period in cycles, less one */
#define UART_DIV_DIV_SHIFT 0 /* read, write: the divider; 867 for 115200 at 100 MHz */
#define UART_DIV_DIV_MASK 0xffff
#define UART_DIV_DIV_WIDTH 16
#define UART_DIV_DIV_RESET 0x363

#endif
