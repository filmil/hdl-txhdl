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
  output led4,
  output uart_tx,
  input uart_rx
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
  // The bus: a request channel out of the core into the router, a
  // response channel back, and a channel each way from the router to
  // the timer and to the serial port. Every channel is a buffer of two,
  // chan69 or chan32, the language's channel as hardware, between the
  // sender's side and the receiver's. The timer raises its interrupt
  // line; the external line has nothing on this board to raise it yet;
  // the serial port's line goes to the board's USB serial bridge.
  wire [68:0] req_tx_data, req_rx_data, treq_tx_data, treq_rx_data;
  wire [68:0] ureq_tx_data, ureq_rx_data;
  wire req_tx_valid, req_tx_ready, req_rx_valid, req_rx_ready;
  wire treq_tx_valid, treq_tx_ready, treq_rx_valid, treq_rx_ready;
  wire ureq_tx_valid, ureq_tx_ready, ureq_rx_valid, ureq_rx_ready;
  wire [31:0] resp_tx_data, resp_rx_data, tresp_tx_data, tresp_rx_data;
  wire [31:0] uresp_tx_data, uresp_rx_data;
  wire resp_tx_valid, resp_tx_ready, resp_rx_valid, resp_rx_ready;
  wire tresp_tx_valid, tresp_tx_ready, tresp_rx_valid, tresp_rx_ready;
  wire uresp_tx_valid, uresp_tx_ready, uresp_rx_valid, uresp_rx_ready;
  wire tirq, uirq;
  vreteno core (.clk(clk), .rst(rst), .irq(uirq), .tirq(tirq),
    .resp_data(resp_rx_data), .resp_valid(resp_rx_valid), .resp_ready(resp_rx_ready),
    .halt(halt), .instr(instr), .wb(wb),
    .req_data(req_tx_data), .req_valid(req_tx_valid), .req_ready(req_tx_ready));
  chan69 req_chan (.clk(clk),
    .tx_data(req_tx_data), .tx_valid(req_tx_valid), .tx_ready(req_tx_ready),
    .rx_data(req_rx_data), .rx_valid(req_rx_valid), .rx_ready(req_rx_ready));
  router rtr (.clk(clk),
    .req_data(req_rx_data), .req_valid(req_rx_valid), .req_ready(req_rx_ready),
    .timer_resp_data(tresp_rx_data), .timer_resp_valid(tresp_rx_valid), .timer_resp_ready(tresp_rx_ready),
    .uart_resp_data(uresp_rx_data), .uart_resp_valid(uresp_rx_valid), .uart_resp_ready(uresp_rx_ready),
    .timer_req_data(treq_tx_data), .timer_req_valid(treq_tx_valid), .timer_req_ready(treq_tx_ready),
    .uart_req_data(ureq_tx_data), .uart_req_valid(ureq_tx_valid), .uart_req_ready(ureq_tx_ready),
    .resp_data(resp_tx_data), .resp_valid(resp_tx_valid), .resp_ready(resp_tx_ready));
  chan32 resp_chan (.clk(clk),
    .tx_data(resp_tx_data), .tx_valid(resp_tx_valid), .tx_ready(resp_tx_ready),
    .rx_data(resp_rx_data), .rx_valid(resp_rx_valid), .rx_ready(resp_rx_ready));
  chan69 treq_chan (.clk(clk),
    .tx_data(treq_tx_data), .tx_valid(treq_tx_valid), .tx_ready(treq_tx_ready),
    .rx_data(treq_rx_data), .rx_valid(treq_rx_valid), .rx_ready(treq_rx_ready));
  timer tim (.clk(clk), .rst(rst),
    .req_data(treq_rx_data), .req_valid(treq_rx_valid), .req_ready(treq_rx_ready),
    .resp_data(tresp_tx_data), .resp_valid(tresp_tx_valid), .resp_ready(tresp_tx_ready),
    .tirq(tirq));
  chan32 tresp_chan (.clk(clk),
    .tx_data(tresp_tx_data), .tx_valid(tresp_tx_valid), .tx_ready(tresp_tx_ready),
    .rx_data(tresp_rx_data), .rx_valid(tresp_rx_valid), .rx_ready(tresp_rx_ready));
  chan69 ureq_chan (.clk(clk),
    .tx_data(ureq_tx_data), .tx_valid(ureq_tx_valid), .tx_ready(ureq_tx_ready),
    .rx_data(ureq_rx_data), .rx_valid(ureq_rx_valid), .rx_ready(ureq_rx_ready));
  uart port (.clk(clk), .rst(rst), .rx(uart_rx),
    .req_data(ureq_rx_data), .req_valid(ureq_rx_valid), .req_ready(ureq_rx_ready),
    .resp_data(uresp_tx_data), .resp_valid(uresp_tx_valid), .resp_ready(uresp_tx_ready),
    .tx(uart_tx), .irq(uirq));
  chan32 uresp_chan (.clk(clk),
    .tx_data(uresp_tx_data), .tx_valid(uresp_tx_valid), .tx_ready(uresp_tx_ready),
    .rx_data(uresp_rx_data), .rx_valid(uresp_rx_valid), .rx_ready(uresp_rx_ready));

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
