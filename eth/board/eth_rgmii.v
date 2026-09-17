// SPDX-License-Identifier: Apache-2.0
//
// RGMII to GMII, for a gigabit PHY such as the KSZ9031 on the Alinx
// AX7A200, on a Xilinx 7-series part.
//
// RGMII sends a GMII byte as two nibbles a cycle: the low nibble while
// the clock is high and the high nibble while it is low, with the valid
// line on the rising edge and valid xor error on the falling one. The
// FPGA's primitives do the two edges: an ODDR per line out and an IDDR
// per line in.
//
// Transmit: the byte and `tx_en` are taken on `clk125`, and the PHY's
// transmit clock is `clk125_90`, the same clock a quarter cycle late,
// so the clock's edges fall in the middle of the data. That is the delay
// RGMII asks of one side or the other; this wrapper supplies it rather
// than relying on the PHY's internal delay.
//
// Receive: the PHY's receive clock comes in through a global buffer and
// clocks the IDDRs, and `rx_clk` is that clock, for the receiving half of
// the MAC. Whether the data is centred on that clock at the pins depends
// on the PHY's receive delay; this wrapper adds none, and it has not yet
// been measured on the board.
//
// Only gigabit works. At 100 or 10 Mbit a PHY sends one nibble per
// clock cycle and the MAC would have to take bytes over two cycles,
// which neither half of the MAC does.
`timescale 1ps / 1ps
module eth_rgmii (
  // The transmit side, GMII, on clk125.
  input clk125,
  input clk125_90,
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
  // The transmit clock: high then low, on the quarter-cycle-late clock.
  ODDR #(.DDR_CLK_EDGE("SAME_EDGE"), .INIT(1'b0), .SRTYPE("SYNC")) txck_oddr (
    .Q(eth_txck), .C(clk125_90), .CE(1'b1),
    .D1(1'b1), .D2(1'b0), .R(1'b0), .S(1'b0)
  );

  // Receive: the PHY's clock, buffered onto the global clock network.
  wire rxck_ibuf;
  IBUF rxck_in (.I(eth_rxck), .O(rxck_ibuf));
  BUFG rxck_bufg (.I(rxck_ibuf), .O(rx_clk));
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
