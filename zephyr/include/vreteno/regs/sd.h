/* The sd register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef SD_REGS_H
#define SD_REGS_H

#define SD_SPAN 0x40
#define SD_CTRL 0x00 /* read, write: the divider, the lines, the interrupt enable */
#define SD_CTRL_DIV_SHIFT 0 /* read, write: cycles a half of the card clock, less one */
#define SD_CTRL_DIV_MASK 0xff
#define SD_CTRL_DIV_WIDTH 8
#define SD_CTRL_DIV_RESET 0x0
#define SD_CTRL_WIDE_SHIFT 8 /* read, write: four data lines rather than one */
#define SD_CTRL_WIDE_MASK 0x100
#define SD_CTRL_WIDE_WIDTH 1
#define SD_CTRL_WIDE_RESET 0x0
#define SD_CTRL_IE_SHIFT 9 /* read, write: a finished command raises the interrupt */
#define SD_CTRL_IE_MASK 0x200
#define SD_CTRL_IE_WIDTH 1
#define SD_CTRL_IE_RESET 0x0
#define SD_CTRL_CLEAR_SHIFT 10 /* write only: written, the buffer's pointers go to zero */
#define SD_CTRL_CLEAR_MASK 0x400
#define SD_CTRL_CLEAR_WIDTH 1
#define SD_CTRL_CLEAR_RESET 0x0
#define SD_CMD 0x04 /* read, write: the command word; written, it starts */
#define SD_CMD_INDEX_SHIFT 0 /* read, write: the command's index */
#define SD_CMD_INDEX_MASK 0x3f
#define SD_CMD_INDEX_WIDTH 6
#define SD_CMD_INDEX_RESET 0x0
#define SD_CMD_RESP_SHIFT 6 /* read, write: the response: none, short, long */
#define SD_CMD_RESP_MASK 0xc0
#define SD_CMD_RESP_WIDTH 2
#define SD_CMD_RESP_RESET 0x0
#define SD_CMD_READ_SHIFT 8 /* read, write: a block comes after the response */
#define SD_CMD_READ_MASK 0x100
#define SD_CMD_READ_WIDTH 1
#define SD_CMD_READ_RESET 0x0
#define SD_CMD_WRITE_SHIFT 9 /* read, write: a block goes after the response */
#define SD_CMD_WRITE_MASK 0x200
#define SD_CMD_WRITE_WIDTH 1
#define SD_CMD_WRITE_RESET 0x0
#define SD_CMD_BUSY_SHIFT 10 /* read, write: wait for the card's busy line afterwards */
#define SD_CMD_BUSY_MASK 0x400
#define SD_CMD_BUSY_WIDTH 1
#define SD_CMD_BUSY_RESET 0x0
#define SD_CMD_NOCRC_SHIFT 11 /* read, write: the response carries no CRC */
#define SD_CMD_NOCRC_MASK 0x800
#define SD_CMD_NOCRC_WIDTH 1
#define SD_CMD_NOCRC_RESET 0x0
#define SD_CMD_ONLY_SHIFT 12 /* read, write: no command, only the block */
#define SD_CMD_ONLY_MASK 0x1000
#define SD_CMD_ONLY_WIDTH 1
#define SD_CMD_ONLY_RESET 0x0
#define SD_CMD_CLOCKS_SHIFT 13 /* read, write: no command and no block: eighty clocks */
#define SD_CMD_CLOCKS_MASK 0x2000
#define SD_CMD_CLOCKS_WIDTH 1
#define SD_CMD_CLOCKS_RESET 0x0
#define SD_ARG 0x08 /* read, write: the argument */
#define SD_STATUS 0x0c /* read, write one to clear: how the last command went */
#define SD_STATUS_BUSY_SHIFT 0 /* read only: a command is running */
#define SD_STATUS_BUSY_MASK 0x1
#define SD_STATUS_BUSY_WIDTH 1
#define SD_STATUS_BUSY_RESET 0x0
#define SD_STATUS_DONE_SHIFT 1 /* read, write one to clear: finished; written, clears the faults too */
#define SD_STATUS_DONE_MASK 0x2
#define SD_STATUS_DONE_WIDTH 1
#define SD_STATUS_DONE_RESET 0x0
#define SD_STATUS_RTIMEOUT_SHIFT 2 /* read only: no response came */
#define SD_STATUS_RTIMEOUT_MASK 0x4
#define SD_STATUS_RTIMEOUT_WIDTH 1
#define SD_STATUS_RTIMEOUT_RESET 0x0
#define SD_STATUS_RCRC_SHIFT 3 /* read only: the response's CRC was wrong */
#define SD_STATUS_RCRC_MASK 0x8
#define SD_STATUS_RCRC_WIDTH 1
#define SD_STATUS_RCRC_RESET 0x0
#define SD_STATUS_DTIMEOUT_SHIFT 4 /* read only: no block came, or the card stayed busy */
#define SD_STATUS_DTIMEOUT_MASK 0x10
#define SD_STATUS_DTIMEOUT_WIDTH 1
#define SD_STATUS_DTIMEOUT_RESET 0x0
#define SD_STATUS_DCRC_SHIFT 5 /* read only: the block's CRC was wrong or refused */
#define SD_STATUS_DCRC_MASK 0x20
#define SD_STATUS_DCRC_WIDTH 1
#define SD_STATUS_DCRC_RESET 0x0
#define SD_STATUS_CRCSTAT_SHIFT 6 /* read only: the card's CRC status token */
#define SD_STATUS_CRCSTAT_MASK 0x1c0
#define SD_STATUS_CRCSTAT_WIDTH 3
#define SD_STATUS_CRCSTAT_RESET 0x0
#define SD_STATUS_DAT0_SHIFT 9 /* read only: the first data line's level */
#define SD_STATUS_DAT0_MASK 0x200
#define SD_STATUS_DAT0_WIDTH 1
#define SD_STATUS_DAT0_RESET 0x0
#define SD_STATUS_INDEX_SHIFT 10 /* read only: the response's index */
#define SD_STATUS_INDEX_MASK 0xfc00
#define SD_STATUS_INDEX_WIDTH 6
#define SD_STATUS_INDEX_RESET 0x0
#define SD_STATUS_WPTR_SHIFT 16 /* read only: where the next word goes in */
#define SD_STATUS_WPTR_MASK 0xff0000
#define SD_STATUS_WPTR_WIDTH 8
#define SD_STATUS_WPTR_RESET 0x0
#define SD_STATUS_RPTR_SHIFT 24 /* read only: where the next word comes out */
#define SD_STATUS_RPTR_MASK 0xff000000
#define SD_STATUS_RPTR_WIDTH 8
#define SD_STATUS_RPTR_RESET 0x0
#define SD_RESP0 0x10 /* read only: the response's first word */
#define SD_RESP1 0x14 /* read only: its second */
#define SD_RESP2 0x18 /* read only: its third */
#define SD_RESP3 0x1c /* read only: its fourth */
#define SD_DATA 0x20 /* read, write: read: the next word out; written: the next word in */

#endif
