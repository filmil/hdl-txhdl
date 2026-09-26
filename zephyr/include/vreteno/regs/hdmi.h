/* The hdmi register map, written by //tools/regmap from
 * its declaration; edit that and not this. */
#ifndef HDMI_REGS_H
#define HDMI_REGS_H

#define HDMI_SPAN 0x10
#define HDMI_STATUS 0x00 /* read only: the raster and the frames shown */
#define HDMI_STATUS_BLANK_SHIFT 0 /* read only: high in vertical blanking */
#define HDMI_STATUS_BLANK_MASK 0x1
#define HDMI_STATUS_BLANK_WIDTH 1
#define HDMI_STATUS_BLANK_RESET 0x0
#define HDMI_STATUS_FRAMES_SHIFT 16 /* read only: frames shown, wrapping */
#define HDMI_STATUS_FRAMES_MASK 0xffff0000
#define HDMI_STATUS_FRAMES_WIDTH 16
#define HDMI_STATUS_FRAMES_RESET 0x0
#define HDMI_CURSOR 0x04 /* read, write: where the next pixel written goes */
#define HDMI_CURSOR_COL_SHIFT 0 /* read, write: the column */
#define HDMI_CURSOR_COL_MASK 0xff
#define HDMI_CURSOR_COL_WIDTH 8
#define HDMI_CURSOR_COL_RESET 0x0
#define HDMI_CURSOR_ROW_SHIFT 8 /* read, write: the row */
#define HDMI_CURSOR_ROW_MASK 0x7f00
#define HDMI_CURSOR_ROW_WIDTH 7
#define HDMI_CURSOR_ROW_RESET 0x0
#define HDMI_PIXEL 0x08 /* write only: a pixel at the cursor, which then moves on */
#define HDMI_PIXEL_COLOUR_SHIFT 0 /* write only: four bits each of red, green and blue */
#define HDMI_PIXEL_COLOUR_MASK 0xfff
#define HDMI_PIXEL_COLOUR_WIDTH 12
#define HDMI_PIXEL_COLOUR_RESET 0x0

#endif
