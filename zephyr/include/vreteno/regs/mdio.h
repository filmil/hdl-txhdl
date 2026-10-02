/* The mdio register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef MDIO_REGS_H
#define MDIO_REGS_H

#define MDIO_SPAN 0x10
#define MDIO_CTRL 0x00 /* read, write: the divider */
#define MDIO_CTRL_DIV_SHIFT 0 /* read, write: half of an MDC cycle is this many cycles, less one */
#define MDIO_CTRL_DIV_MASK 0xff
#define MDIO_CTRL_DIV_WIDTH 8
#define MDIO_CTRL_DIV_RESET 0x0
#define MDIO_CMD 0x04 /* write only: one frame; written, it starts */
#define MDIO_CMD_REGAD_SHIFT 0 /* write only: the register */
#define MDIO_CMD_REGAD_MASK 0x1f
#define MDIO_CMD_REGAD_WIDTH 5
#define MDIO_CMD_REGAD_RESET 0x0
#define MDIO_CMD_PHYAD_SHIFT 5 /* write only: the PHY's address */
#define MDIO_CMD_PHYAD_MASK 0x3e0
#define MDIO_CMD_PHYAD_WIDTH 5
#define MDIO_CMD_PHYAD_RESET 0x0
#define MDIO_CMD_WRITE_SHIFT 10 /* write only: write the word rather than read */
#define MDIO_CMD_WRITE_MASK 0x400
#define MDIO_CMD_WRITE_WIDTH 1
#define MDIO_CMD_WRITE_RESET 0x0
#define MDIO_CMD_WORD_SHIFT 16 /* write only: the word to write */
#define MDIO_CMD_WORD_MASK 0xffff0000
#define MDIO_CMD_WORD_WIDTH 16
#define MDIO_CMD_WORD_RESET 0x0
#define MDIO_DATA 0x08 /* read only: the word the last read took */
#define MDIO_STATE 0x0c /* read, write one to clear: busy and done */
#define MDIO_STATE_BUSY_SHIFT 0 /* read only: a frame is running */
#define MDIO_STATE_BUSY_MASK 0x1
#define MDIO_STATE_BUSY_WIDTH 1
#define MDIO_STATE_BUSY_RESET 0x0
#define MDIO_STATE_FIRED_SHIFT 1 /* read, write one to clear: a frame has finished */
#define MDIO_STATE_FIRED_MASK 0x2
#define MDIO_STATE_FIRED_WIDTH 1
#define MDIO_STATE_FIRED_RESET 0x0

#endif
