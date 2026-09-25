// SPDX-License-Identifier: Apache-2.0
//
// The HDMI demonstration on the Alinx AX7A200B: a test picture at 640 by
// 480 and 60 Hz, through the board's SiI9134 encoder.
//
// Everything that computes is lowered: `hdmi_video`, the video
// peripheral with the picture in its framebuffer, and `hdmi_i2c`, the
// master that resets and configures the chip. Around them is what a
// board needs and a lowered unit cannot say: the pixel clock, the clock
// sent to the chip, and the two open-drain I2C lines.
//
// The clock: the board's 200 MHz input into an MMCM, divided by ten and
// multiplied by 63 for a 1260 MHz oscillator, and divided by 50 for
// 25.2 MHz, within half a per cent of the mode's 25.175 MHz.
//
// The chip's clock is the pixel clock turned over by an ODDR, so the
// chip's rising edge falls in the middle of each pixel the FPGA drives
// on its own rising edge.
//
// Nothing writes the framebuffer here, so the peripheral's AXI-Lite
// inputs are held idle and its answers are taken and dropped.
//
// The LEDs are lit when driven low: the MMCM locked, the chip
// configured, and a byte the chip did not acknowledge.
`timescale 1ps / 1ps
module hdmi_demo (
  input sys_clk_p,
  input sys_clk_n,
  output led1,
  output led2,
  output led3,
  output hdmi_nreset,
  // The same reset again, on the ball the other board revision wires it
  // to. The vendor's own demonstration for this board drives both, one
  // named for revision 1.0, and which ball reaches the chip depends on
  // which board this is. Driving both costs a pin that nothing else
  // wants and removes the question. See issue #197.
  output hdmi_nreset_alt,
  output hdmi_clk,
  output hdmi_hs,
  output hdmi_vs,
  output hdmi_de,
  output [23:0] hdmi_d,
  output hdmi_scl,
  inout hdmi_sda
);
  // The pixel clock.
  wire clk200, fb, fb_buf, locked, mmcm25, clk;
  IBUFDS clkin (.I(sys_clk_p), .IB(sys_clk_n), .O(clk200));
  MMCME2_BASE #(
    .CLKIN1_PERIOD(5.0),
    .DIVCLK_DIVIDE(10),
    .CLKFBOUT_MULT_F(63.0),
    .CLKOUT0_DIVIDE_F(50.0)
  ) mmcm (
    .CLKIN1(clk200),
    .CLKFBIN(fb_buf),
    .CLKFBOUT(fb),
    .CLKOUT0(mmcm25),
    .CLKOUT0B(), .CLKOUT1(), .CLKOUT1B(), .CLKOUT2(), .CLKOUT2B(),
    .CLKOUT3(), .CLKOUT3B(), .CLKOUT4(), .CLKOUT5(), .CLKOUT6(),
    .CLKFBOUTB(),
    .LOCKED(locked),
    .PWRDWN(1'b0),
    .RST(1'b0)
  );
  BUFG fb_bufg (.I(fb), .O(fb_buf));
  BUFG clk_bufg (.I(mmcm25), .O(clk));

  // The reset for the two units, on their own clock: the clock
  // manager's lock, low until the pixel clock is good, through two
  // flip-flops, so that the release lands on an edge of the clock it
  // releases. The units run from their reset arms rather than from the
  // power-on values alone (issue 509).
  (* ASYNC_REG = "TRUE" *) reg [1:0] rst_sync = 2'b11;
  always @(posedge clk) rst_sync <= {rst_sync[0], ~locked};
  wire rst = rst_sync[1];

  // The video.
  hdmi_video video (
    .clk(clk),
    .rst(rst),
    .bus_aw_data(35'd0), .bus_aw_valid(1'b0), .bus_aw_ready(),
    .bus_ar_data(35'd0), .bus_ar_valid(1'b0), .bus_ar_ready(),
    .bus_w_data(36'd0), .bus_w_valid(1'b0), .bus_w_ready(),
    .bus_b_data(), .bus_b_valid(), .bus_b_ready(1'b1),
    .bus_r_data(), .bus_r_valid(), .bus_r_ready(1'b1),
    .rgb(hdmi_d), .hsync(hdmi_hs), .vsync(hdmi_vs), .de(hdmi_de)
  );
  ODDR #(.DDR_CLK_EDGE("SAME_EDGE"), .INIT(1'b0), .SRTYPE("SYNC")) clk_oddr (
    .Q(hdmi_clk), .C(clk), .CE(1'b1),
    .D1(1'b0), .D2(1'b1), .R(1'b0), .S(1'b0)
  );

  // The chip's reset and configuration.
  wire scl_low, sda_low, done, failed;
  hdmi_i2c master (
    .clk(clk),
    .rst(rst),
    .sda_in(hdmi_sda),
    .nreset(hdmi_nreset), .scl_low(scl_low), .sda_low(sda_low),
    .done(done), .failed(failed)
  );
  assign hdmi_scl = scl_low ? 1'b0 : 1'bz;
  assign hdmi_sda = sda_low ? 1'b0 : 1'bz;

  assign led1 = !locked;
  assign led2 = !done;
  assign hdmi_nreset_alt = hdmi_nreset;
  assign led3 = !failed;
endmodule
