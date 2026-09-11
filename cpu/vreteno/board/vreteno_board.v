// SPDX-License-Identifier: Apache-2.0
// The Vreteno core on the Alinx AX7A200 board. This is the one file of
// the design written by hand rather than lowered, and it holds what a
// board needs and a core does not: the 200 MHz differential clock
// brought down to the 100 MHz the core meets, a reset from the board's
// button synchronised into that clock and held until the clock is
// locked, and four LEDs, lit when driven low, that show the core
// halted, a heartbeat, the reset released and the clock locked.
`timescale 1ns/1ps
module vreteno_board(
  input sys_clk_p,
  input sys_clk_n,
  input reset_n,
  output led1,
  output led2,
  output led3,
  output led4
);
  // The clock: 200 MHz in, 100 MHz out, through an MMCM.
  wire clk200;
  IBUFDS clkin (.I(sys_clk_p), .IB(sys_clk_n), .O(clk200));
  wire fb, fb_buf, clk100_raw, clk, locked;
  MMCME2_BASE #(
    .CLKIN1_PERIOD(5.0),
    .CLKFBOUT_MULT_F(5.0),
    .CLKOUT0_DIVIDE_F(10.0),
    .DIVCLK_DIVIDE(1)
  ) mmcm (
    .CLKIN1(clk200),
    .CLKFBIN(fb_buf),
    .CLKFBOUT(fb),
    .CLKOUT0(clk100_raw),
    .CLKOUT0B(), .CLKOUT1(), .CLKOUT1B(), .CLKOUT2(), .CLKOUT2B(),
    .CLKOUT3(), .CLKOUT3B(), .CLKOUT4(), .CLKOUT5(), .CLKOUT6(),
    .CLKFBOUTB(),
    .LOCKED(locked),
    .PWRDWN(1'b0),
    .RST(1'b0)
  );
  BUFG fbbuf (.I(fb), .O(fb_buf));
  BUFG clkbuf (.I(clk100_raw), .O(clk));

  // The reset: the button is active low; held while the clock is not
  // locked; two flip-flops into the core's clock.
  reg [1:0] sync = 2'b11;
  always @(posedge clk) sync <= {sync[0], ~reset_n | ~locked};
  wire rst = sync[1];

  // The core.
  wire halt;
  wire [31:0] instr;
  wire [37:0] wb;
  vreteno core (.clk(clk), .rst(rst), .halt(halt), .instr(instr), .wb(wb));

  // The heartbeat: bit 25 of a counter at 100 MHz toggles three times a
  // second.
  reg [25:0] beat = 0;
  always @(posedge clk) beat <= beat + 1;

  // LEDs are lit when driven low.
  assign led1 = ~halt;
  assign led2 = ~beat[25];
  assign led3 = rst;
  assign led4 = ~locked;
endmodule
