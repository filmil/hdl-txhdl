// SPDX-License-Identifier: Apache-2.0
//
// A channel from one clock to another: an asynchronous FIFO with the
// valid/ready handshake the lowered units' channel ports have on each
// side. A word moves when valid and ready are both high at its clock's
// rising edge, as between two lowered units in one clock.
//
// The pointers count in Gray code and each crosses to the other clock
// through two flip-flops, so a pointer seen from the far side is at
// worst a cycle or two stale, never torn. The writing side sees the
// FIFO full a little late and the reading side sees it empty a little
// late, which only delays a word; neither loses one. `AW` is the
// address width, so the FIFO holds `1 << AW` words.
//
// Each side has a reset on its own clock, active high and already
// synchronised to that clock. Each side also sees the other's reset,
// through two flip-flops of its own clock, and holds itself while
// either is on: its pointer and its view of the other's at zero, and
// neither ready nor valid. So a reset of either side empties the FIFO,
// and nothing moves until both sides are out of it, whichever releases
// first; a word in flight when the reset came is dropped rather than
// handed to a reader that was reset and never asked for it (issue
// 1325).
//
// The hand-written board designs that have two clocks and a channel
// between them read this file: //eth's echo crosses the PHY's receive
// clock to the transmit clock with it, and //flagship crosses the
// core's AXI-Lite, a channel each way per AXI channel, to the pixel
// clock. A lowered design crosses with ChanCdc in //lib/parts instead,
// as the Vreteno board does between the cable's clock and its own.
`timescale 1ps / 1ps
module chan_cdc #(
  parameter W = 9,
  parameter AW = 4
) (
  // The writing side.
  input wr_clk,
  input wr_rst,
  input [W-1:0] wr_data,
  input wr_valid,
  output wr_ready,
  // The reading side.
  input rd_clk,
  input rd_rst,
  output [W-1:0] rd_data,
  output rd_valid,
  input rd_ready
);
  reg [W-1:0] mem [0:(1 << AW) - 1];

  // Binary and Gray pointers, one more bit than the address, so full
  // and empty differ.
  reg [AW:0] wbin = 0, wgray = 0;
  reg [AW:0] rbin = 0, rgray = 0;
  // Each side's view of the other's Gray pointer, through two flops.
  (* ASYNC_REG = "TRUE" *) reg [AW:0] rgray_w1 = 0, rgray_w2 = 0;
  (* ASYNC_REG = "TRUE" *) reg [AW:0] wgray_r1 = 0, wgray_r2 = 0;
  // Each side's view of the other's reset, through two flops, starting
  // held so that neither side moves before it has seen the other.
  (* ASYNC_REG = "TRUE" *) reg [1:0] rrst_w = 2'b11;
  (* ASYNC_REG = "TRUE" *) reg [1:0] wrst_r = 2'b11;
  wire wr_hold = wr_rst | rrst_w[1];
  wire rd_hold = rd_rst | wrst_r[1];

  wire full = wgray == {~rgray_w2[AW:AW-1], rgray_w2[AW-2:0]};
  wire empty = rgray == wgray_r2;
  assign wr_ready = !full && !wr_hold;
  assign rd_valid = !empty && !rd_hold;
  assign rd_data = mem[rbin[AW-1:0]];

  wire push = wr_valid && wr_ready;
  wire [AW:0] wbin_next = wbin + 1;
  always @(posedge wr_clk) begin
    rrst_w <= {rrst_w[0], rd_rst};
    if (wr_hold) begin
      wbin <= 0;
      wgray <= 0;
      rgray_w1 <= 0;
      rgray_w2 <= 0;
    end else begin
      if (push) begin
        mem[wbin[AW-1:0]] <= wr_data;
        wbin <= wbin_next;
        wgray <= wbin_next ^ (wbin_next >> 1);
      end
      rgray_w1 <= rgray;
      rgray_w2 <= rgray_w1;
    end
  end

  wire pop = rd_ready && rd_valid;
  wire [AW:0] rbin_next = rbin + 1;
  always @(posedge rd_clk) begin
    wrst_r <= {wrst_r[0], wr_rst};
    if (rd_hold) begin
      rbin <= 0;
      rgray <= 0;
      wgray_r1 <= 0;
      wgray_r2 <= 0;
    end else begin
      if (pop) begin
        rbin <= rbin_next;
        rgray <= rbin_next ^ (rbin_next >> 1);
      end
      wgray_r1 <= wgray;
      wgray_r2 <= wgray_r1;
    end
  end
endmodule
