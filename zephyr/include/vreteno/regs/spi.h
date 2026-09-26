/* The spi register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef SPI_REGS_H
#define SPI_REGS_H

#define SPI_SPAN 0x10
#define SPI_CTRL 0x00 /* read, write: the divider, the mode, the select, the enable */
#define SPI_CTRL_DIV_SHIFT 0 /* read, write: a half of a bit is this many cycles, less one */
#define SPI_CTRL_DIV_MASK 0xff
#define SPI_CTRL_DIV_WIDTH 8
#define SPI_CTRL_DIV_RESET 0x0
#define SPI_CTRL_CPOL_SHIFT 8 /* read, write: the level the clock idles at */
#define SPI_CTRL_CPOL_MASK 0x100
#define SPI_CTRL_CPOL_WIDTH 1
#define SPI_CTRL_CPOL_RESET 0x0
#define SPI_CTRL_CPHA_SHIFT 9 /* read, write: the trailing edge carries the bit */
#define SPI_CTRL_CPHA_MASK 0x200
#define SPI_CTRL_CPHA_WIDTH 1
#define SPI_CTRL_CPHA_RESET 0x0
#define SPI_CTRL_SEL_SHIFT 10 /* read, write: the chip select, held while set */
#define SPI_CTRL_SEL_MASK 0x400
#define SPI_CTRL_SEL_WIDTH 1
#define SPI_CTRL_SEL_RESET 0x0
#define SPI_CTRL_IE_SHIFT 11 /* read, write: a finished transfer raises the interrupt */
#define SPI_CTRL_IE_MASK 0x800
#define SPI_CTRL_IE_WIDTH 1
#define SPI_CTRL_IE_RESET 0x0
#define SPI_DATA 0x04 /* read, write: written: the byte to send; read: the byte that came */
#define SPI_STATE 0x08 /* read, write one to clear: whether a transfer runs, and whether one finished */
#define SPI_STATE_BUSY_SHIFT 0 /* read only: a transfer is running */
#define SPI_STATE_BUSY_MASK 0x1
#define SPI_STATE_BUSY_WIDTH 1
#define SPI_STATE_BUSY_RESET 0x0
#define SPI_STATE_FIRED_SHIFT 1 /* read, write one to clear: a transfer has finished */
#define SPI_STATE_FIRED_MASK 0x2
#define SPI_STATE_FIRED_WIDTH 1
#define SPI_STATE_FIRED_RESET 0x0

#endif
