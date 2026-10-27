// SPDX-License-Identifier: Apache-2.0
//
// FireSim on the AX7A200B with Ethernet as the host link (issue 1931).
//
// A program on the machine at the other end of the board's Ethernet
// cable reads and writes the simulator's control registers in frames.
// The receive half of the MAC takes a frame off the PHY on the PHY's
// clock; the bytes cross to the simulator's 50 MHz clock, where
// HostLink turns the frame into an ask, LiteMaster performs it on
// AXI-Lite, and a few lines below make that one beat of the AXI4
// control port of Golden Gate's FPGATop. The answer goes back the
// same way, its bytes crossing to 125 MHz for the transmit half.
//
// The crossings are on the two byte streams, nine bits wide, rather
// than on the five AXI channels: the MAC stores whole frames, so
// feeding it at 50 MHz costs nothing but time.
//
// The clocks and the PHY are //eth's echo design's.
`timescale 1ps / 1ps
module firesim_eth_top (
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
  // 200 MHz in; 125 MHz for the PHY, 50 MHz for the simulator.
  wire clk200, fb, fb_buf, locked;
  wire mmcm125, mmcm50;
  wire clk125, clk50;
  IBUFDS clkin (.I(sys_clk_p), .IB(sys_clk_n), .O(clk200));
  MMCME2_BASE #(
    .CLKIN1_PERIOD(5.0),
    .DIVCLK_DIVIDE(1),
    .CLKFBOUT_MULT_F(5.0),
    .CLKOUT0_DIVIDE_F(8.0),
    .CLKOUT1_DIVIDE(20)
  ) mmcm (
    .CLKIN1(clk200),
    .CLKFBIN(fb_buf),
    .CLKFBOUT(fb),
    .CLKOUT0(mmcm125),
    .CLKOUT1(mmcm50),
    .CLKOUT0B(), .CLKOUT1B(), .CLKOUT2(), .CLKOUT2B(), .CLKOUT3(),
    .CLKOUT3B(), .CLKOUT4(), .CLKOUT5(), .CLKOUT6(), .CLKFBOUTB(),
    .LOCKED(locked),
    .PWRDWN(1'b0),
    .RST(1'b0)
  );
  BUFG fb_bufg (.I(fb), .O(fb_buf));
  BUFG clk125_bufg (.I(mmcm125), .O(clk125));
  BUFG clk50_bufg (.I(mmcm50), .O(clk50));

  // A reset per clock, held until the MMCM locks.
  wire rx_clk;
  (* ASYNC_REG = "TRUE" *) reg [1:0] rx_rst_sync = 2'b11;
  (* ASYNC_REG = "TRUE" *) reg [1:0] tx_rst_sync = 2'b11;
  (* ASYNC_REG = "TRUE" *) reg [1:0] h_rst_sync = 2'b11;
  always @(posedge rx_clk) rx_rst_sync <= {rx_rst_sync[0], ~locked};
  always @(posedge clk125) tx_rst_sync <= {tx_rst_sync[0], ~locked};
  always @(posedge clk50) h_rst_sync <= {h_rst_sync[0], ~locked};
  wire rx_rst = rx_rst_sync[1];
  wire tx_rst = tx_rst_sync[1];
  wire h_rst = h_rst_sync[1];

  // The PHY.
  wire [7:0] txd, rxd;
  wire tx_en, rx_dv, rx_er;
  eth_rgmii rgmii (
    .clk125(clk125), .txd(txd), .tx_en(tx_en),
    .rx_clk(rx_clk), .rxd(rxd), .rx_dv(rx_dv), .rx_er(rx_er),
    .eth_txck(eth_txck), .eth_txctl(eth_txctl), .eth_txd(eth_txd),
    .eth_rxck(eth_rxck), .eth_rxctl(eth_rxctl), .eth_rxd(eth_rxd)
  );
  assign eth_reset_n = locked;
  assign eth_mdc = 1'b1;
  assign eth_mdio = 1'bz;

  // Frames in, on the PHY's clock, and across to the simulator's.
  wire [8:0] rx_data, h_rx_data;
  wire rx_valid, rx_ready, h_rx_valid, h_rx_ready;
  eth_rx mac_rx (
    .clk(rx_clk),
    .rst(rx_rst),
    .rxd(rxd), .rx_dv(rx_dv), .rx_er(rx_er),
    .rx_data(rx_data), .rx_valid(rx_valid), .rx_ready(rx_ready)
  );
  chan_cdc #(.W(9), .AW(4)) rx_cross (
    .wr_rst(rx_rst), .rd_rst(h_rst),
    .wr_clk(rx_clk), .wr_data(rx_data), .wr_valid(rx_valid),
    .wr_ready(rx_ready),
    .rd_clk(clk50), .rd_data(h_rx_data), .rd_valid(h_rx_valid),
    .rd_ready(h_rx_ready)
  );

  // The link and the master, at 50 MHz, as one lowered unit: joined
  // inside it, their channels are buffered; joined by wires here, each
  // channel's valid would depend on its own ready (a combinational loop).
  // The AXI-Lite channels, each packed as its struct, first field in
  // the high bits: LiteAddr {addr[32], prot[3]}, LiteW {data[32],
  // strb[4]}, LiteB {resp[2]}, LiteR {data[32], resp[2]}.
  wire [8:0] h_tx_data;
  wire h_tx_valid, h_tx_ready;
  wire [34:0] aw_data, ar_data;
  wire [35:0] w_data;
  wire [1:0] b_data;
  wire [33:0] r_data;
  wire aw_valid, aw_ready, ar_valid, ar_ready, w_valid, w_ready;
  wire b_valid, b_ready, r_valid, r_ready;
  host_bridge bridge (
    .clk(clk50), .rst(h_rst),
    .rx_data(h_rx_data), .rx_valid(h_rx_valid), .rx_ready(h_rx_ready),
    .tx_data(h_tx_data), .tx_valid(h_tx_valid), .tx_ready(h_tx_ready),
    .bus_aw_data(aw_data), .bus_aw_valid(aw_valid), .bus_aw_ready(aw_ready),
    .bus_ar_data(ar_data), .bus_ar_valid(ar_valid), .bus_ar_ready(ar_ready),
    .bus_w_data(w_data), .bus_w_valid(w_valid), .bus_w_ready(w_ready),
    .bus_b_data(b_data), .bus_b_valid(b_valid), .bus_b_ready(b_ready),
    .bus_r_data(r_data), .bus_r_valid(r_valid), .bus_r_ready(r_ready)
  );

  // The simulator. Each AXI-Lite transaction is one beat of AXI4: ID 0,
  // length 0 (one beat), size 2 (four bytes), an incrementing burst.
  // The control space is 32 MB, so the address's low 25 bits.
  wire [1:0] b_resp, r_resp;
  wire [31:0] r_word;
  FPGATop sim (
    .clock(clk50),
    .reset(h_rst),
    .ctrl_aw_ready(aw_ready),
    .ctrl_aw_valid(aw_valid),
    .ctrl_aw_bits_addr(aw_data[27:3]),
    .ctrl_aw_bits_len(8'd0),
    .ctrl_aw_bits_size(3'd2),
    .ctrl_aw_bits_burst(2'd1),
    .ctrl_aw_bits_lock(1'b0),
    .ctrl_aw_bits_cache(4'd0),
    .ctrl_aw_bits_prot(aw_data[2:0]),
    .ctrl_aw_bits_qos(4'd0),
    .ctrl_aw_bits_region(4'd0),
    .ctrl_aw_bits_id(12'd0),
    .ctrl_aw_bits_user(1'b0),
    .ctrl_w_ready(w_ready),
    .ctrl_w_valid(w_valid),
    .ctrl_w_bits_data(w_data[35:4]),
    .ctrl_w_bits_last(1'b1),
    .ctrl_w_bits_id(12'd0),
    .ctrl_w_bits_strb(w_data[3:0]),
    .ctrl_w_bits_user(1'b0),
    .ctrl_b_ready(b_ready),
    .ctrl_b_valid(b_valid),
    .ctrl_b_bits_resp(b_resp),
    .ctrl_b_bits_id(),
    .ctrl_b_bits_user(),
    .ctrl_ar_ready(ar_ready),
    .ctrl_ar_valid(ar_valid),
    .ctrl_ar_bits_addr(ar_data[27:3]),
    .ctrl_ar_bits_len(8'd0),
    .ctrl_ar_bits_size(3'd2),
    .ctrl_ar_bits_burst(2'd1),
    .ctrl_ar_bits_lock(1'b0),
    .ctrl_ar_bits_cache(4'd0),
    .ctrl_ar_bits_prot(ar_data[2:0]),
    .ctrl_ar_bits_qos(4'd0),
    .ctrl_ar_bits_region(4'd0),
    .ctrl_ar_bits_id(12'd0),
    .ctrl_ar_bits_user(1'b0),
    .ctrl_r_ready(r_ready),
    .ctrl_r_valid(r_valid),
    .ctrl_r_bits_resp(r_resp),
    .ctrl_r_bits_data(r_word),
    .ctrl_r_bits_last(),
    .ctrl_r_bits_id(),
    .ctrl_r_bits_user()
  );
  assign b_data = b_resp;
  assign r_data = {r_word, r_resp};

  // Answers out: across to 125 MHz and onto the PHY.
  wire [8:0] tx_data;
  wire tx_valid, tx_ready;
  chan_cdc #(.W(9), .AW(4)) tx_cross (
    .wr_rst(h_rst), .rd_rst(tx_rst),
    .wr_clk(clk50), .wr_data(h_tx_data), .wr_valid(h_tx_valid),
    .wr_ready(h_tx_ready),
    .rd_clk(clk125), .rd_data(tx_data), .rd_valid(tx_valid),
    .rd_ready(tx_ready)
  );
  eth_tx mac_tx (
    .clk(clk125),
    .rst(tx_rst),
    .tx_data(tx_data), .tx_valid(tx_valid), .tx_ready(tx_ready),
    .txd(txd), .tx_en(tx_en)
  );

  // The lights: locked, a frame byte taken in, an answer sent.
  reg [24:0] seen_ask = 0, seen_tx = 0;
  always @(posedge clk50)
    seen_ask <= (h_rx_valid && h_rx_ready) ? ~25'd0
              : (seen_ask != 0 ? seen_ask - 1 : 0);
  always @(posedge clk125)
    seen_tx <= tx_en ? ~25'd0 : (seen_tx != 0 ? seen_tx - 1 : 0);
  assign led1 = !locked;
  assign led2 = seen_ask == 0;
  assign led3 = seen_tx == 0;
endmodule
