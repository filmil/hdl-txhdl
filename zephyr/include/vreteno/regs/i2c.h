/* The i2c register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef I2C_REGS_H
#define I2C_REGS_H

#define I2C_SPAN 0x10
#define I2C_CTRL 0x00 /* read, write: the divider and the interrupt enable */
#define I2C_CTRL_DIV_SHIFT 0 /* read, write: a quarter of a bit is this many cycles, less one */
#define I2C_CTRL_DIV_MASK 0xffff
#define I2C_CTRL_DIV_WIDTH 16
#define I2C_CTRL_DIV_RESET 0x0
#define I2C_CTRL_IE_SHIFT 16 /* read, write: a finished command raises the interrupt */
#define I2C_CTRL_IE_MASK 0x10000
#define I2C_CTRL_IE_WIDTH 1
#define I2C_CTRL_IE_RESET 0x0
#define I2C_CMD 0x04 /* write only: one piece of a transaction; written, it starts */
#define I2C_CMD_START_SHIFT 0 /* write only: send a start, or a repeated start */
#define I2C_CMD_START_MASK 0x1
#define I2C_CMD_START_WIDTH 1
#define I2C_CMD_START_RESET 0x0
#define I2C_CMD_STOP_SHIFT 1 /* write only: send a stop when the byte is done */
#define I2C_CMD_STOP_MASK 0x2
#define I2C_CMD_STOP_WIDTH 1
#define I2C_CMD_STOP_RESET 0x0
#define I2C_CMD_WRITE_SHIFT 2 /* write only: write the byte */
#define I2C_CMD_WRITE_MASK 0x4
#define I2C_CMD_WRITE_WIDTH 1
#define I2C_CMD_WRITE_RESET 0x0
#define I2C_CMD_READ_SHIFT 3 /* write only: read a byte */
#define I2C_CMD_READ_MASK 0x8
#define I2C_CMD_READ_WIDTH 1
#define I2C_CMD_READ_RESET 0x0
#define I2C_CMD_NACK_SHIFT 4 /* write only: answer a read with a NACK rather than an ACK */
#define I2C_CMD_NACK_MASK 0x10
#define I2C_CMD_NACK_WIDTH 1
#define I2C_CMD_NACK_RESET 0x0
#define I2C_CMD_BYTE_SHIFT 8 /* write only: the byte to write */
#define I2C_CMD_BYTE_MASK 0xff00
#define I2C_CMD_BYTE_WIDTH 8
#define I2C_CMD_BYTE_RESET 0x0
#define I2C_DATA 0x08 /* read only: the byte a read took */
#define I2C_STATE 0x0c /* read, write one to clear: busy, done, not acknowledged, arbitration lost */
#define I2C_STATE_BUSY_SHIFT 0 /* read only: a command is running */
#define I2C_STATE_BUSY_MASK 0x1
#define I2C_STATE_BUSY_WIDTH 1
#define I2C_STATE_BUSY_RESET 0x0
#define I2C_STATE_FIRED_SHIFT 1 /* read, write one to clear: a command has finished */
#define I2C_STATE_FIRED_MASK 0x2
#define I2C_STATE_FIRED_WIDTH 1
#define I2C_STATE_FIRED_RESET 0x0
#define I2C_STATE_NACK_SHIFT 2 /* read, write one to clear: the device did not acknowledge the byte */
#define I2C_STATE_NACK_MASK 0x4
#define I2C_STATE_NACK_WIDTH 1
#define I2C_STATE_NACK_RESET 0x0
#define I2C_STATE_LOST_SHIFT 3 /* read, write one to clear: another master won the bus */
#define I2C_STATE_LOST_MASK 0x8
#define I2C_STATE_LOST_WIDTH 1
#define I2C_STATE_LOST_RESET 0x0

#endif
