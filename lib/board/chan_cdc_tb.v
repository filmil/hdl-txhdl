// SPDX-License-Identifier: Apache-2.0
// chan_cdc's resets (issue 1325): words cross in order between two
// clocks, and a reset of either side, or of both released in either
// order, leaves nothing from before it for the reader, with nothing
// moving while either side is held.
`timescale 1ps / 1ps
module chan_cdc_tb;
  reg wclk = 0, rclk = 0;
  always #5000 wclk = ~wclk;   // 100 MHz, as the core's clock
  always #19841 rclk = ~rclk;  // 25.2 MHz, as the pixel clock
  reg wrst = 1, rrst = 1;
  reg [15:0] wdata = 0;
  reg wvalid = 0;
  reg rready = 0;
  wire wready, rvalid;
  wire [15:0] rdata;
  chan_cdc #(.W(16), .AW(3)) dut (
    .wr_clk(wclk), .wr_rst(wrst), .wr_data(wdata), .wr_valid(wvalid),
    .wr_ready(wready),
    .rd_clk(rclk), .rd_rst(rrst), .rd_data(rdata), .rd_valid(rvalid),
    .rd_ready(rready)
  );

  // One word in, waiting for room.
  task push(input [15:0] w);
    begin
      // Ready changes only at the writer's rising edge, so its value
      // at the falling edge is the one the next rising edge takes.
      @(negedge wclk);
      wdata = w;
      wvalid = 1;
      while (!wready) @(negedge wclk);
      @(posedge wclk);
      #1 wvalid = 0;
    end
  endtask

  // One word out, which must be `w`.
  task pop(input [15:0] w);
    begin
      @(negedge rclk);
      rready = 1;
      while (!rvalid) @(negedge rclk);
      if (rdata !== w) begin
        $display("read %h, expected %h at %0t", rdata, w, $time);
        $fatal(1, "a word out of order");
      end
      @(posedge rclk);
      #1 rready = 0;
    end
  endtask

  // Nothing offered to the reader for `n` of its cycles.
  task none(input integer n);
    integer i;
    begin
      for (i = 0; i < n; i = i + 1) begin
        @(posedge rclk);
        if (rvalid) $fatal(1, "a stale word %h was offered", rdata);
      end
    end
  endtask

  // Both held while either side is in reset.
  task held(input integer n);
    integer i;
    begin
      for (i = 0; i < n; i = i + 1) begin
        @(posedge wclk);
        if (wready) $fatal(1, "the writer was ready under a reset");
        if (rvalid) $fatal(1, "the reader was offered a word under a reset");
      end
    end
  endtask

  integer k;
  initial begin
    repeat (4) @(posedge rclk);
    wrst = 0;
    rrst = 0;
    repeat (4) @(posedge rclk);

    // Words cross in order.
    for (k = 0; k < 20; k = k + 1) begin
      push(16'h1000 + k[15:0]);
      pop(16'h1000 + k[15:0]);
    end
    $display("in order: 20 words");

    // Five words left in it, then both reset, the writer released
    // first and the reader long after.
    for (k = 0; k < 5; k = k + 1) push(16'h2000 + k[15:0]);
    repeat (6) @(posedge rclk);
    wrst = 1;
    rrst = 1;
    held(20);
    wrst = 0;
    held(20);
    rrst = 0;
    none(8);
    for (k = 0; k < 6; k = k + 1) push(16'h3000 + k[15:0]);
    for (k = 0; k < 6; k = k + 1) pop(16'h3000 + k[15:0]);
    none(4);
    $display("both reset, writer first: no stale word");

    // The same, the reader released first.
    for (k = 0; k < 5; k = k + 1) push(16'h4000 + k[15:0]);
    repeat (6) @(posedge rclk);
    wrst = 1;
    rrst = 1;
    held(20);
    rrst = 0;
    held(20);
    wrst = 0;
    none(8);
    for (k = 0; k < 6; k = k + 1) push(16'h5000 + k[15:0]);
    for (k = 0; k < 6; k = k + 1) pop(16'h5000 + k[15:0]);
    none(4);
    $display("both reset, reader first: no stale word");

    // The reader alone reset: what was in it is gone, and the writer
    // waits until the reader is out.
    for (k = 0; k < 5; k = k + 1) push(16'h6000 + k[15:0]);
    repeat (6) @(posedge rclk);
    rrst = 1;
    repeat (3) @(posedge wclk);
    held(20);
    rrst = 0;
    none(8);
    for (k = 0; k < 6; k = k + 1) push(16'h7000 + k[15:0]);
    for (k = 0; k < 6; k = k + 1) pop(16'h7000 + k[15:0]);
    $display("reader reset alone: no stale word");

    // The writer alone reset.
    for (k = 0; k < 5; k = k + 1) push(16'h8000 + k[15:0]);
    repeat (6) @(posedge rclk);
    wrst = 1;
    repeat (3) @(posedge rclk);
    held(20);
    wrst = 0;
    none(8);
    for (k = 0; k < 6; k = k + 1) push(16'h9000 + k[15:0]);
    for (k = 0; k < 6; k = k + 1) pop(16'h9000 + k[15:0]);
    $display("writer reset alone: no stale word");

    // Full: the writer waits, then everything comes in order.
    for (k = 0; k < 8; k = k + 1) push(16'ha000 + k[15:0]);
    repeat (4) @(posedge rclk);
    if (wready) $fatal(1, "eight words in a FIFO of eight, and room");
    for (k = 0; k < 8; k = k + 1) pop(16'ha000 + k[15:0]);
    $display("full and drained: 8 words");
    $display("chan_cdc: ok");
    $finish;
  end
endmodule
