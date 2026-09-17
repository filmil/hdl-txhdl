// SPDX-License-Identifier: Apache-2.0
//
// An Ethernet echo on the Alinx AX7A200: every good frame the PHY
// receives is sent back out as it came, with a fresh check sequence.
// The design to bring the port up with before a core is behind it.
//
// Everything that computes is the two lowered halves of the MAC,
// `eth_rx` and `eth_tx`, which `#[lower]` writes for `EthRx` and
// `EthTx`. Around them is what a board needs and a lowered unit cannot
// say: the clocks, the PHY's two edges in `eth_rgmii`, and the crossing
// from the receive clock to the transmit clock in `eth_cdc`.
//
// The clocks: the board's 200 MHz input into an MMCM, times five for a
// 1000 MHz oscillator, divided by eight for 125 MHz and again for the
// same clock a quarter cycle late, which the PHY's transmit clock is.
// The receive half runs on the clock the PHY recovers from the line.
//
// The LEDs are lit when driven low: the MMCM locked, a frame received,
// and a frame sent, the last two held for a while so a person sees them.
// The PHY's reset is released once the MMCM has locked; the management
// interface is left idle, so the PHY runs on its strapped defaults.
`timescale 1ps / 1ps
module eth_echo (
  input sys_clk_p,
  input sys_clk_n,
  output led1,
  output led2,
  output led3,
  output eth_txck,
  output eth_txctl,
  output [3:0] eth_txd,
  input eth_rxck,
  input eth_rxctl,
  input [3:0] eth_rxd,
  output eth_reset_n,
  output eth_mdc,
  inout eth_mdio
);
  // The clocks.
  wire clk200, fb, fb_buf, locked;
  wire mmcm125, mmcm125_90;
  wire clk125, clk125_90;
  IBUFDS clkin (.I(sys_clk_p), .IB(sys_clk_n), .O(clk200));
  MMCME2_BASE #(
    .CLKIN1_PERIOD(5.0),
    .DIVCLK_DIVIDE(1),
    .CLKFBOUT_MULT_F(5.0),
    .CLKOUT0_DIVIDE_F(8.0),
    .CLKOUT1_DIVIDE(8),
    .CLKOUT1_PHASE(90.0)
  ) mmcm (
    .CLKIN1(clk200),
    .CLKFBIN(fb_buf),
    .CLKFBOUT(fb),
    .CLKOUT0(mmcm125),
    .CLKOUT1(mmcm125_90),
    .CLKOUT0B(), .CLKOUT1B(), .CLKOUT2(), .CLKOUT2B(), .CLKOUT3(),
    .CLKOUT3B(), .CLKOUT4(), .CLKOUT5(), .CLKOUT6(), .CLKFBOUTB(),
    .LOCKED(locked),
    .PWRDWN(1'b0),
    .RST(1'b0)
  );
  BUFG fb_bufg (.I(fb), .O(fb_buf));
  BUFG clk125_bufg (.I(mmcm125), .O(clk125));
  BUFG clk125_90_bufg (.I(mmcm125_90), .O(clk125_90));

  // The PHY.
  wire rx_clk;
  wire [7:0] txd, rxd;
  wire tx_en, rx_dv, rx_er;
  eth_rgmii rgmii (
    .clk125(clk125), .clk125_90(clk125_90), .txd(txd), .tx_en(tx_en),
    .rx_clk(rx_clk), .rxd(rxd), .rx_dv(rx_dv), .rx_er(rx_er),
    .eth_txck(eth_txck), .eth_txctl(eth_txctl), .eth_txd(eth_txd),
    .eth_rxck(eth_rxck), .eth_rxctl(eth_rxctl), .eth_rxd(eth_rxd)
  );
  assign eth_reset_n = locked;
  assign eth_mdc = 1'b1;
  assign eth_mdio = 1'bz;

  // The receiving half, on the PHY's clock.
  wire [8:0] rx_data;
  wire rx_valid, rx_ready;
  eth_rx mac_rx (
    .clk(rx_clk),
    .rxd(rxd), .rx_dv(rx_dv), .rx_er(rx_er),
    .rx_data(rx_data), .rx_valid(rx_valid), .rx_ready(rx_ready)
  );

  // From the receive clock to the transmit clock.
  wire [8:0] tx_data;
  wire tx_valid, tx_ready;
  eth_cdc #(.W(9), .AW(4)) crossing (
    .wr_clk(rx_clk), .wr_data(rx_data), .wr_valid(rx_valid),
    .wr_ready(rx_ready),
    .rd_clk(clk125), .rd_data(tx_data), .rd_valid(tx_valid),
    .rd_ready(tx_ready)
  );

  // The sending half, on the transmit clock.
  eth_tx mac_tx (
    .clk(clk125),
    .tx_data(tx_data), .tx_valid(tx_valid), .tx_ready(tx_ready),
    .txd(txd), .tx_en(tx_en)
  );

  // The LEDs: a frame's last byte in or out stretches its light for
  // about a fifth of a second at 125 MHz.
  reg [24:0] seen_rx = 0, seen_tx = 0;
  always @(posedge rx_clk)
    seen_rx <= (rx_valid && rx_ready && rx_data[0]) ? ~25'd0
             : (seen_rx != 0 ? seen_rx - 1 : 0);
  always @(posedge clk125)
    seen_tx <= tx_en ? ~25'd0 : (seen_tx != 0 ? seen_tx - 1 : 0);
  assign led1 = !locked;
  assign led2 = seen_rx == 0;
  assign led3 = seen_tx == 0;
endmodule
