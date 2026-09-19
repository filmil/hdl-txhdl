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
// Every board design that has two clocks and a channel between them
// reads this file: //eth's echo crosses the PHY's receive clock to the
// transmit clock with it, and //flagship crosses the core's AXI-Lite,
// a channel each way per AXI channel, to the pixel clock.
`timescale 1ps / 1ps
module chan_cdc #(
  parameter W = 9,
  parameter AW = 4
) (
  // The writing side.
  input wr_clk,
  input [W-1:0] wr_data,
  input wr_valid,
  output wr_ready,
  // The reading side.
  input rd_clk,
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

  wire full = wgray == {~rgray_w2[AW:AW-1], rgray_w2[AW-2:0]};
  wire empty = rgray == wgray_r2;
  assign wr_ready = !full;
  assign rd_valid = !empty;
  assign rd_data = mem[rbin[AW-1:0]];

  wire push = wr_valid && !full;
  wire [AW:0] wbin_next = wbin + 1;
  always @(posedge wr_clk) begin
    if (push) begin
      mem[wbin[AW-1:0]] <= wr_data;
      wbin <= wbin_next;
      wgray <= wbin_next ^ (wbin_next >> 1);
    end
    rgray_w1 <= rgray;
    rgray_w2 <= rgray_w1;
  end

  wire pop = rd_ready && !empty;
  wire [AW:0] rbin_next = rbin + 1;
  always @(posedge rd_clk) begin
    if (pop) begin
      rbin <= rbin_next;
      rgray <= rbin_next ^ (rbin_next >> 1);
    end
    wgray_r1 <= wgray;
    wgray_r2 <= wgray_r1;
  end
endmodule
