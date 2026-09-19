// SPDX-License-Identifier: Apache-2.0
//
// RGMII to GMII, for a gigabit PHY such as the JL2121 on the Alinx
// AX7A200B, on a Xilinx 7-series part.
//
// RGMII sends a GMII byte as two nibbles a cycle: the low nibble while
// the clock is high and the high nibble while it is low, with the valid
// line on the rising edge and valid xor error on the falling one. The
// FPGA's primitives do the two edges: an ODDR per line out and an IDDR
// per line in.
//
// Transmit: the byte and `tx_en` are taken on `clk125`, and the PHY's
// transmit clock is the same clock through the same kind of register,
// so the clock's edges leave the FPGA aligned with the data's. RGMII
// asks one side or the other to delay the clock into the middle of the
// data, and on this board the PHY does it. This wrapper used to supply
// a quarter cycle of its own as well, and the two delays together put
// the PHY's sampling edge where the data changes: the board received
// every frame and the host got most of them back with a bad check
// sequence, about one bit in ten thousand wrong. The vendor's own design
// for this board sends the clock edge-aligned, and pings through it
// without loss. See issue #231.
//
// Receive: the PHY's receive clock comes in through a global buffer and
// clocks the IDDRs, and `rx_clk` is that clock, for the receiving half of
// the MAC. RGMII asks one side to put the clock in the middle of the
// data, and the JL2121 on this board does not: with no delay here, the
// board linked at gigabit and the MAC accepted no frame at all, which is
// issue #231.
//
// So the receive clock goes through an `IDELAYE2` before its buffer,
// tapped to about two nanoseconds, which is a quarter of the 8 ns bit
// period, and the sampling edge then falls inside the data rather than
// where it changes. The delay is on the clock and not on the five data
// lines on purpose: one delay element moves the sampling point and
// leaves the lines' alignment to each other alone, where five of them
// would add each element's own error between the bits and narrow the
// eye. Delaying the data was tried first and let a third of the frames
// through.
//
// The taps are calibrated by an `IDELAYCTRL` on the board's 200 MHz
// clock, which the design already makes, and `RX_DELAY_TAPS` is a
// parameter so that the value can be moved without reading this file.
//
// Only gigabit works. At 100 or 10 Mbit a PHY sends one nibble per
// clock cycle and the MAC would have to take bytes over two cycles,
// which neither half of the MAC does.
`timescale 1ps / 1ps
module eth_rgmii #(
  // How far the receive clock is shifted, in degrees of its own cycle.
  // A quarter of 8 ns is 2 ns, which is the middle of a data bit.
  parameter real RX_CLOCK_PHASE = 90.0
) (
  // The transmit side, GMII, on clk125.
  input clk125,
  input [7:0] txd,
  input tx_en,
  // The receive side, GMII, on rx_clk.
  output rx_clk,
  output [7:0] rxd,
  output rx_dv,
  output rx_er,
  // The PHY.
  output eth_txck,
  output eth_txctl,
  output [3:0] eth_txd,
  input eth_rxck,
  input eth_rxctl,
  input [3:0] eth_rxd
);
  // Transmit: low nibble on the rising edge, high nibble on the falling.
  genvar i;
  generate
    for (i = 0; i < 4; i = i + 1) begin : txd_oddr
      ODDR #(.DDR_CLK_EDGE("SAME_EDGE"), .INIT(1'b0), .SRTYPE("SYNC")) o (
        .Q(eth_txd[i]), .C(clk125), .CE(1'b1),
        .D1(txd[i]), .D2(txd[i + 4]), .R(1'b0), .S(1'b0)
      );
    end
  endgenerate
  // The control line: tx_en on both edges, since the MAC sends no errors.
  ODDR #(.DDR_CLK_EDGE("SAME_EDGE"), .INIT(1'b0), .SRTYPE("SYNC")) txctl_oddr (
    .Q(eth_txctl), .C(clk125), .CE(1'b1),
    .D1(tx_en), .D2(tx_en), .R(1'b0), .S(1'b0)
  );
  // The transmit clock: high then low, on the same clock as the data,
  // so the two leave the pins aligned and the PHY adds the delay.
  ODDR #(.DDR_CLK_EDGE("SAME_EDGE"), .INIT(1'b0), .SRTYPE("SYNC")) txck_oddr (
    .Q(eth_txck), .C(clk125), .CE(1'b1),
    .D1(1'b1), .D2(1'b0), .R(1'b0), .S(1'b0)
  );

  // Receive: the PHY's clock through an MMCM that shifts it a quarter
  // of a cycle, which is 2 ns at 125 MHz, and then onto the global clock
  // network. The shift puts the sampling edge in the middle of the data
  // rather than where it changes.
  //
  // The shift is made once, on the clock, rather than by delaying the
  // five data lines with an `IDELAYE2` each. Both were measured on the
  // board under #231: five delays of 20 to 31 taps let between a quarter
  // and a third of the frames through, because each delay element brings
  // its own error and the bits drift apart; a delay on the clock alone,
  // in the other direction, let none through.
  wire rxck_ibuf, rxck_shifted, rxck_fb, rxck_fb_buf, rx_locked;
  IBUF rxck_in (.I(eth_rxck), .O(rxck_ibuf));
  MMCME2_BASE #(
    .CLKIN1_PERIOD(8.0),
    .CLKFBOUT_MULT_F(8.0),
    .DIVCLK_DIVIDE(1),
    .CLKOUT0_DIVIDE_F(8.0),
    .CLKOUT0_PHASE(RX_CLOCK_PHASE)
  ) rxmmcm (
    .CLKIN1(rxck_ibuf),
    .CLKFBIN(rxck_fb_buf),
    .CLKFBOUT(rxck_fb),
    .CLKOUT0(rxck_shifted),
    .LOCKED(rx_locked),
    .RST(1'b0),
    .PWRDWN(1'b0)
  );
  BUFG rxck_fb_bufg (.I(rxck_fb), .O(rxck_fb_buf));
  BUFG rxck_bufg (.I(rxck_shifted), .O(rx_clk));

  // Both nibbles and both halves of the control line, presented
  // together on the rising edge.
  wire [3:0] rx_lo, rx_hi;
  wire ctl_rise, ctl_fall;
  generate
    for (i = 0; i < 4; i = i + 1) begin : rxd_iddr
      IDDR #(.DDR_CLK_EDGE("SAME_EDGE_PIPELINED"), .INIT_Q1(1'b0),
             .INIT_Q2(1'b0), .SRTYPE("SYNC")) d (
        .Q1(rx_lo[i]), .Q2(rx_hi[i]), .C(rx_clk), .CE(1'b1),
        .D(eth_rxd[i]), .R(1'b0), .S(1'b0)
      );
    end
  endgenerate
  IDDR #(.DDR_CLK_EDGE("SAME_EDGE_PIPELINED"), .INIT_Q1(1'b0),
         .INIT_Q2(1'b0), .SRTYPE("SYNC")) rxctl_iddr (
    .Q1(ctl_rise), .Q2(ctl_fall), .C(rx_clk), .CE(1'b1),
    .D(eth_rxctl), .R(1'b0), .S(1'b0)
  );
  assign rxd = {rx_hi, rx_lo};
  assign rx_dv = ctl_rise;
  assign rx_er = ctl_rise ^ ctl_fall;
endmodule
