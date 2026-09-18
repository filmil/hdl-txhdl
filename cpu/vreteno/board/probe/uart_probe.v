// SPDX-License-Identifier: Apache-2.0
// A bitstream that does nothing but talk on the serial port.
//
// The board's design halts with every LED on and the host's serial port
// stays silent, which is issue #195. Nothing so far separates a design
// that never drove the line from a board path that does not carry it,
// because every serial run so far has been of one design. This one has
// no core, no memory and no bus: the board's 200 MHz clock, a counter,
// and a transmitter that says one line a second.
//
// The pins, the I/O standards and the clock are the ones the board's own
// design uses, so a line here and silence there says the fault is inside
// that design, and silence here says it is the path or the host.
`timescale 1ps / 1ps
module uart_probe (
  input sys_clk_p,
  input sys_clk_n,
  input reset_n,
  output led1,
  output led2,
  output led3,
  output led4,
  output reg uart_tx
);
  // The board's differential 200 MHz, as the design takes it.
  wire clk;
  IBUFDS #(.DIFF_TERM("FALSE"), .IBUF_LOW_PWR("FALSE")) clk_buf (
    .I(sys_clk_p), .IB(sys_clk_n), .O(clk)
  );

  // 200 MHz over 115200 baud is 1736 cycles a bit, which is 0.006 per
  // cent fast and well inside what a receiver takes.
  localparam integer BIT_CYCLES = 1736;
  // A line a second, so a watcher that attaches at any moment sees one.
  localparam integer GAP_CYCLES = 200000000;
  // The message: `uart probe` and a line ending.
  localparam integer LEN = 12;

  reg [7:0] text [0:LEN-1];
  initial begin
    text[0] = "u";
    text[1] = "a";
    text[2] = "r";
    text[3] = "t";
    text[4] = " ";
    text[5] = "p";
    text[6] = "r";
    text[7] = "o";
    text[8] = "b";
    text[9] = "e";
    text[10] = 8'h0d;
    text[11] = 8'h0a;
  end

  // The button, brought into this clock by two flip-flops.
  reg rst_1 = 1'b1;
  reg rst = 1'b1;
  always @(posedge clk) begin
    rst_1 <= ~reset_n;
    rst <= rst_1;
  end

  // A frame is a low start bit, eight data bits least significant
  // first, and a high stop bit, each BIT_CYCLES long. The shifter holds
  // the data bits and the stop bit, and shifts ones in behind them, so
  // the line idles high whatever happens next.
  reg [8:0] shifter = 9'h1ff;
  reg [3:0] bits = 0;
  reg [11:0] tick = 0;
  reg [27:0] gap = 0;
  reg [3:0] index = 0;
  reg sending = 0;

  always @(posedge clk) begin
    if (rst) begin
      uart_tx <= 1'b1;
      shifter <= 9'h1ff;
      bits <= 0;
      tick <= 0;
      gap <= 0;
      index <= 0;
      sending <= 0;
    end else if (sending) begin
      if (tick == BIT_CYCLES - 1) begin
        tick <= 0;
        uart_tx <= shifter[0];
        shifter <= {1'b1, shifter[8:1]};
        // Nine bit times after the start bit: eight data, one stop.
        if (bits == 9) begin
          bits <= 0;
          sending <= 0;
          index <= (index == LEN - 1) ? 4'd0 : index + 4'd1;
        end else begin
          bits <= bits + 4'd1;
        end
      end else begin
        tick <= tick + 12'd1;
      end
    end else begin
      // The characters of a line follow one another; between lines the
      // counter holds at the first character for a second.
      if (index != 0 || gap == GAP_CYCLES - 1) begin
        gap <= 0;
        sending <= 1'b1;
        tick <= 0;
        bits <= 0;
        uart_tx <= 1'b0;
        shifter <= {1'b1, text[index]};
      end else begin
        gap <= gap + 28'd1;
        uart_tx <= 1'b1;
      end
    end
  end

  // The LEDs are lit when driven low: a heartbeat, so somebody at the
  // board can tell this bitstream from the design's, and a second LED
  // lit while a character is going out.
  reg [26:0] beat = 0;
  always @(posedge clk) beat <= beat + 27'd1;

  assign led1 = ~beat[26];
  assign led2 = ~sending;
  assign led3 = 1'b1;
  assign led4 = 1'b1;
endmodule
