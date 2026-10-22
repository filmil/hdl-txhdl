// SPDX-License-Identifier: Apache-2.0
// TxHDL's two channel buffers driven by a sender that does not wait for
// ready, as AXI requires of a source (#1871). The sender holds its
// valid and its word until the word is taken, and pauses between words
// now and then; the receiver's ready comes and goes. Each word must
// arrive once, in order. Before #1871 a buffer that was full stored an
// offer in the cycle the receiver freed a place, while the sender saw
// ready low and offered the same word again: a duplicate.
module offer_lane #(parameter UNREG = 0, parameter [15:0] SEED = 16'h1)(
  input clk,
  input rst,
  // At the end: no new offer, and the receiver always ready, so the
  // buffer drains.
  input stop,
  output reg [31:0] sent,
  output reg [31:0] got
);
  reg [7:0] word;
  reg tv;
  wire tr;
  wire [7:0] rd;
  wire rv;
  // A Fibonacci shift register: bits 0 and 3 decide whether the sender
  // offers after a word, bits 1 and 2 whether the receiver is ready.
  reg [15:0] lfsr;
  wire rr = stop | lfsr[1] | lfsr[2];
  wire go = ~stop & (lfsr[0] | lfsr[3]);
  generate
    if (UNREG) begin : u
      pair_u_txhdl_chan_u #(.W(8)) ch(
        .clk(clk), .rst(rst),
        .tx_data(word), .tx_valid(tv), .tx_ready(tr),
        .rx_data(rd), .rx_valid(rv), .rx_ready(rr));
    end else begin : r
      pair_u_txhdl_chan #(.W(8)) ch(
        .clk(clk), .rst(rst),
        .tx_data(word), .tx_valid(tv), .tx_ready(tr),
        .rx_data(rd), .rx_valid(rv), .rx_ready(rr));
    end
  endgenerate
  always @(posedge clk) begin
    if (rst) begin
      word <= 0;
      tv <= 0;
      sent <= 0;
      got <= 0;
      lfsr <= SEED;
    end else begin
      lfsr <= {lfsr[14:0], lfsr[15] ^ lfsr[13] ^ lfsr[12] ^ lfsr[10]};
      // The sender: an offer stays up, with the same word, until taken.
      if (tv & tr) begin
        word <= word + 1;
        sent <= sent + 1;
        tv <= go;
      end else if (!tv) begin
        tv <= go;
      end
      // The receiver: each word it takes is the next one.
      if (rv & rr) begin
        if (rd != got[7:0]) begin
          $display("offer (%s): word %0d was %0d",
                   UNREG ? "unregistered" : "registered", got, rd);
          $fatal(1);
        end
        got <= got + 1;
      end
    end
  end
endmodule

module offer_tb;
  reg clk = 0;
  reg rst = 1;
  reg stop = 0;
  wire [31:0] rsent, rgot, usent, ugot;
  offer_lane #(.UNREG(0), .SEED(16'hace1)) reg_lane(
    .clk(clk), .rst(rst), .stop(stop), .sent(rsent), .got(rgot));
  offer_lane #(.UNREG(1), .SEED(16'hbeef)) unreg_lane(
    .clk(clk), .rst(rst), .stop(stop), .sent(usent), .got(ugot));
  integer cycle;
  initial begin
    // 4000 cycles of traffic, then ten to drain: the last offer is
    // taken and the buffer empties.
    for (cycle = 0; cycle < 4010; cycle = cycle + 1) begin
      #5 clk = 1;
      #5 clk = 0;
      if (cycle == 2) rst = 0;
      if (cycle == 3999) stop = 1;
    end
    // Nothing lost and nothing extra: once drained, every word taken
    // from the sender has reached the receiver.
    if (rsent != rgot || usent != ugot) begin
      $display("offer: sent %0d and %0d, got %0d and %0d",
               rsent, usent, rgot, ugot);
      $fatal(1);
    end
    // About 2800 words pass in each; fewer than 2500 is a buffer that
    // stalls.
    if (rgot < 2500 || ugot < 2500) begin
      $display("offer: only %0d and %0d words passed", rgot, ugot);
      $fatal(1);
    end
    $display("offer: %0d and %0d words passed, each once, in order",
             rgot, ugot);
    $finish;
  end
endmodule
