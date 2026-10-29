// SPDX-License-Identifier: Apache-2.0
//
// FireSim's Rocket design on the AX7A200B, with Ethernet as the host link
// and the board's DDR3 as the target's memory (issue 1988).
//
// This is firesim_eth_top.v (FireSim's GCD example, issue 1931) with two
// more ports of Golden Gate's FPGATop joined:
//
// - The control port and the CPU-managed stream port both take frames
//   from the Ethernet port. rocket_link (rocket_netlist.rs) splits them
//   by EtherType: 0x88b5 to HostLink and LiteMaster, the control
//   port's AXI-Lite host, and 0x88b6 to StreamBridge, a host on the
//   128-bit cpu_managed port, one beat per frame.
// - mem_0, the target's DRAM, goes to the MIG: across from the
//   simulator's 50 MHz to the MIG's 100 MHz user clock in an AXI clock
//   converter, at FPGATop's widths (64-bit data, 16-bit IDs, a 30-bit
//   address), which the MIG takes as they are.
//
// The simulator is held in reset until the MIG has calibrated.
//
// A reset frame from the driver (issue 2007) starts the MIG's power-on
// reset again. FireSim expects a fresh simulator for each run, and the
// MIG's calibration then holds the clock converter, the simulator and
// the link in reset, as at power-on. The frame comes when no answer is
// going out: the driver sends it first, and waits for the board to
// answer again.
//
// The clocks: the board's 200 MHz in, through one IBUFDS, to the MIG,
// unbuffered as //ddr3's controller takes it, and to an MMCM for the
// PHY's 125 MHz and the simulator's 50 MHz.
`timescale 1ps / 1ps
module rocket_eth_top (
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
  inout eth_mdio,
  output [14:0] ddr3_a,
  output [2:0] ddr3_ba,
  output ddr3_ras,
  output ddr3_cas,
  output ddr3_we,
  output ddr3_s0,
  output ddr3_cke,
  output ddr3_odt,
  output ddr3_reset,
  output ddr3_clk_p,
  output ddr3_clk_n,
  output [3:0] ddr3_dm,
  inout [31:0] ddr3_dq_p,
  inout [3:0] ddr3_dqs_p,
  inout [3:0] ddr3_dqs_n
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

  // The MIG's reset: a pulse at power-on, counted on the board's clock,
  // as the Vreteno board makes it, and again after each reset frame.
  // The link's `reset` is high for one cycle of the simulator's clock;
  // held for seventeen, from a register, it crosses to the board's
  // clock in two flip-flops, and its rising edge starts the count again.
  wire link_reset;
  reg [3:0] soft = 4'd0;
  reg soft_on = 1'b0;
  always @(posedge clk50) begin
    soft <= link_reset ? 4'hf : (soft != 4'd0 ? soft - 4'd1 : 4'd0);
    soft_on <= link_reset | (soft != 4'd0);
  end
  (* ASYNC_REG = "TRUE" *) reg [1:0] soft_sync = 2'b00;
  reg soft_was = 1'b0;
  always @(posedge clk200) begin
    soft_sync <= {soft_sync[0], soft_on};
    soft_was <= soft_sync[1];
  end
  wire again = soft_sync[1] & ~soft_was;
  reg [7:0] por = 8'd0;
  always @(posedge clk200)
    if (again) por <= 8'd0;
    else if (por != 8'hff) por <= por + 1;
  wire mig_sys_rst = (por != 8'hff);
  wire ui_clk, ui_rst, calib;

  // A reset per clock, held until the MMCM locks. The simulator's waits
  // on the memory's calibration too.
  wire rx_clk;
  (* ASYNC_REG = "TRUE" *) reg [1:0] rx_rst_sync = 2'b11;
  (* ASYNC_REG = "TRUE" *) reg [1:0] tx_rst_sync = 2'b11;
  (* ASYNC_REG = "TRUE" *) reg [1:0] sim_rst_sync = 2'b11;
  always @(posedge rx_clk) rx_rst_sync <= {rx_rst_sync[0], ~locked};
  always @(posedge clk125) tx_rst_sync <= {tx_rst_sync[0], ~locked};
  always @(posedge clk50)
    sim_rst_sync <= {sim_rst_sync[0], ~locked | ~calib | ui_rst};
  wire rx_rst = rx_rst_sync[1];
  wire tx_rst = tx_rst_sync[1];
  wire sim_rst = sim_rst_sync[1];
  reg ui_aresetn = 1'b0;
  always @(posedge ui_clk) ui_aresetn <= ~ui_rst;

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
    .wr_rst(rx_rst), .rd_rst(sim_rst),
    .wr_clk(rx_clk), .wr_data(rx_data), .wr_valid(rx_valid),
    .wr_ready(rx_ready),
    .rd_clk(clk50), .rd_data(h_rx_data), .rd_valid(h_rx_valid),
    .rd_ready(h_rx_ready)
  );

  // The host link at 50 MHz, one lowered unit. Each channel is packed as
  // its struct, first field in the high bits:
  //   LiteAddr {addr[32], prot[3]}, LiteW {data[32], strb[4]},
  //   LiteB {resp[2]}, LiteR {data[32], resp[2]};
  //   Addr {id[4], addr[64], len[8], size[3], burst[2], lock, cache[4],
  //         prot[3], qos[4], region[4]}, W {data[128], strb[16], last},
  //   B {id[4], resp[2]}, R {id[4], data[128], resp[2], last}.
  // The link's bursts are one beat; its IDs are zero.
  wire [8:0] h_tx_data;
  wire h_tx_valid, h_tx_ready;
  wire [34:0] c_aw, c_ar;
  wire [35:0] c_w;
  wire [1:0] c_b;
  wire [33:0] c_r;
  wire c_aw_valid, c_aw_ready, c_ar_valid, c_ar_ready, c_w_valid, c_w_ready;
  wire c_b_valid, c_b_ready, c_r_valid, c_r_ready;
  wire [96:0] s_aw, s_ar;
  wire [144:0] s_w;
  wire [5:0] s_b;
  wire [134:0] s_r;
  wire s_aw_valid, s_aw_ready, s_ar_valid, s_ar_ready, s_w_valid, s_w_ready;
  wire s_b_valid, s_b_ready, s_r_valid, s_r_ready;
  // The link waits for the simulator's reset too, so no frame becomes a
  // transaction on a port of FPGATop while it is held in reset; the
  // driver sends again what is lost meanwhile.
  rocket_link link (
    .clk(clk50), .rst(sim_rst),
    .rx_data(h_rx_data), .rx_valid(h_rx_valid), .rx_ready(h_rx_ready),
    .tx_data(h_tx_data), .tx_valid(h_tx_valid), .tx_ready(h_tx_ready),
    .bus_aw_data(c_aw), .bus_aw_valid(c_aw_valid), .bus_aw_ready(c_aw_ready),
    .bus_ar_data(c_ar), .bus_ar_valid(c_ar_valid), .bus_ar_ready(c_ar_ready),
    .bus_w_data(c_w), .bus_w_valid(c_w_valid), .bus_w_ready(c_w_ready),
    .bus_b_data(c_b), .bus_b_valid(c_b_valid), .bus_b_ready(c_b_ready),
    .bus_r_data(c_r), .bus_r_valid(c_r_valid), .bus_r_ready(c_r_ready),
    .aw_data(s_aw), .aw_valid(s_aw_valid), .aw_ready(s_aw_ready),
    .ar_data(s_ar), .ar_valid(s_ar_valid), .ar_ready(s_ar_ready),
    .w_data(s_w), .w_valid(s_w_valid), .w_ready(s_w_ready),
    .b_data(s_b), .b_valid(s_b_valid), .b_ready(s_b_ready),
    .r_data(s_r), .r_valid(s_r_valid), .r_ready(s_r_ready),
    .reset(link_reset)
  );

  // The target's memory, on the simulator's side of the clock converter.
  wire m_awvalid, m_awready, m_wvalid, m_wready, m_bvalid, m_bready;
  wire m_arvalid, m_arready, m_rvalid, m_rready, m_wlast, m_rlast;
  wire [15:0] m_awid, m_arid, m_bid, m_rid;
  wire [29:0] m_awaddr, m_araddr;
  wire [7:0] m_awlen, m_arlen, m_wstrb;
  wire [2:0] m_awsize, m_arsize, m_awprot, m_arprot;
  wire [1:0] m_awburst, m_arburst, m_bresp, m_rresp;
  wire m_awlock, m_arlock;
  wire [3:0] m_awcache, m_arcache, m_awqos, m_arqos;
  wire [63:0] m_wdata, m_rdata;

  // The simulator. The control port as in firesim_eth_top.v: each
  // AXI-Lite transaction one beat of AXI4, ID 0, four bytes, and the
  // control space's low 25 bits of the address.
  wire [1:0] c_b_resp, c_r_resp;
  wire [31:0] c_r_word;
  wire [3:0] s_b_id, s_r_id;
  wire [1:0] s_b_resp, s_r_resp;
  wire [127:0] s_r_word;
  wire s_r_last;
  FPGATop sim (
    .clock(clk50),
    .reset(sim_rst),
    .ctrl_aw_ready(c_aw_ready),
    .ctrl_aw_valid(c_aw_valid),
    .ctrl_aw_bits_addr(c_aw[27:3]),
    .ctrl_aw_bits_len(8'd0),
    .ctrl_aw_bits_size(3'd2),
    .ctrl_aw_bits_burst(2'd1),
    .ctrl_aw_bits_lock(1'b0),
    .ctrl_aw_bits_cache(4'd0),
    .ctrl_aw_bits_prot(c_aw[2:0]),
    .ctrl_aw_bits_qos(4'd0),
    .ctrl_aw_bits_region(4'd0),
    .ctrl_aw_bits_id(12'd0),
    .ctrl_aw_bits_user(1'b0),
    .ctrl_w_ready(c_w_ready),
    .ctrl_w_valid(c_w_valid),
    .ctrl_w_bits_data(c_w[35:4]),
    .ctrl_w_bits_last(1'b1),
    .ctrl_w_bits_id(12'd0),
    .ctrl_w_bits_strb(c_w[3:0]),
    .ctrl_w_bits_user(1'b0),
    .ctrl_b_ready(c_b_ready),
    .ctrl_b_valid(c_b_valid),
    .ctrl_b_bits_resp(c_b_resp),
    .ctrl_b_bits_id(),
    .ctrl_b_bits_user(),
    .ctrl_ar_ready(c_ar_ready),
    .ctrl_ar_valid(c_ar_valid),
    .ctrl_ar_bits_addr(c_ar[27:3]),
    .ctrl_ar_bits_len(8'd0),
    .ctrl_ar_bits_size(3'd2),
    .ctrl_ar_bits_burst(2'd1),
    .ctrl_ar_bits_lock(1'b0),
    .ctrl_ar_bits_cache(4'd0),
    .ctrl_ar_bits_prot(c_ar[2:0]),
    .ctrl_ar_bits_qos(4'd0),
    .ctrl_ar_bits_region(4'd0),
    .ctrl_ar_bits_id(12'd0),
    .ctrl_ar_bits_user(1'b0),
    .ctrl_r_ready(c_r_ready),
    .ctrl_r_valid(c_r_valid),
    .ctrl_r_bits_resp(c_r_resp),
    .ctrl_r_bits_data(c_r_word),
    .ctrl_r_bits_last(),
    .ctrl_r_bits_id(),
    .ctrl_r_bits_user(),
    // The streams' port, from StreamBridge's packed channels.
    .cpu_managed_axi4_aw_ready(s_aw_ready),
    .cpu_managed_axi4_aw_valid(s_aw_valid),
    .cpu_managed_axi4_aw_bits_id(s_aw[96:93]),
    .cpu_managed_axi4_aw_bits_addr(s_aw[92:29]),
    .cpu_managed_axi4_aw_bits_len(s_aw[28:21]),
    .cpu_managed_axi4_aw_bits_size(s_aw[20:18]),
    .cpu_managed_axi4_aw_bits_burst(s_aw[17:16]),
    .cpu_managed_axi4_aw_bits_lock(s_aw[15]),
    .cpu_managed_axi4_aw_bits_cache(s_aw[14:11]),
    .cpu_managed_axi4_aw_bits_prot(s_aw[10:8]),
    .cpu_managed_axi4_aw_bits_qos(s_aw[7:4]),
    .cpu_managed_axi4_w_ready(s_w_ready),
    .cpu_managed_axi4_w_valid(s_w_valid),
    .cpu_managed_axi4_w_bits_data(s_w[144:17]),
    .cpu_managed_axi4_w_bits_strb(s_w[16:1]),
    .cpu_managed_axi4_w_bits_last(s_w[0]),
    .cpu_managed_axi4_b_ready(s_b_ready),
    .cpu_managed_axi4_b_valid(s_b_valid),
    .cpu_managed_axi4_b_bits_id(s_b_id),
    .cpu_managed_axi4_b_bits_resp(s_b_resp),
    .cpu_managed_axi4_ar_ready(s_ar_ready),
    .cpu_managed_axi4_ar_valid(s_ar_valid),
    .cpu_managed_axi4_ar_bits_id(s_ar[96:93]),
    .cpu_managed_axi4_ar_bits_addr(s_ar[92:29]),
    .cpu_managed_axi4_ar_bits_len(s_ar[28:21]),
    .cpu_managed_axi4_ar_bits_size(s_ar[20:18]),
    .cpu_managed_axi4_ar_bits_burst(s_ar[17:16]),
    .cpu_managed_axi4_ar_bits_lock(s_ar[15]),
    .cpu_managed_axi4_ar_bits_cache(s_ar[14:11]),
    .cpu_managed_axi4_ar_bits_prot(s_ar[10:8]),
    .cpu_managed_axi4_ar_bits_qos(s_ar[7:4]),
    .cpu_managed_axi4_r_ready(s_r_ready),
    .cpu_managed_axi4_r_valid(s_r_valid),
    .cpu_managed_axi4_r_bits_id(s_r_id),
    .cpu_managed_axi4_r_bits_data(s_r_word),
    .cpu_managed_axi4_r_bits_resp(s_r_resp),
    .cpu_managed_axi4_r_bits_last(s_r_last),
    // The target's DRAM.
    .mem_0_aw_ready(m_awready),
    .mem_0_aw_valid(m_awvalid),
    .mem_0_aw_bits_id(m_awid),
    .mem_0_aw_bits_addr(m_awaddr),
    .mem_0_aw_bits_len(m_awlen),
    .mem_0_aw_bits_size(m_awsize),
    .mem_0_aw_bits_burst(m_awburst),
    .mem_0_aw_bits_lock(m_awlock),
    .mem_0_aw_bits_cache(m_awcache),
    .mem_0_aw_bits_prot(m_awprot),
    .mem_0_aw_bits_qos(m_awqos),
    .mem_0_w_ready(m_wready),
    .mem_0_w_valid(m_wvalid),
    .mem_0_w_bits_data(m_wdata),
    .mem_0_w_bits_strb(m_wstrb),
    .mem_0_w_bits_last(m_wlast),
    .mem_0_b_ready(m_bready),
    .mem_0_b_valid(m_bvalid),
    .mem_0_b_bits_id(m_bid),
    .mem_0_b_bits_resp(m_bresp),
    .mem_0_ar_ready(m_arready),
    .mem_0_ar_valid(m_arvalid),
    .mem_0_ar_bits_id(m_arid),
    .mem_0_ar_bits_addr(m_araddr),
    .mem_0_ar_bits_len(m_arlen),
    .mem_0_ar_bits_size(m_arsize),
    .mem_0_ar_bits_burst(m_arburst),
    .mem_0_ar_bits_lock(m_arlock),
    .mem_0_ar_bits_cache(m_arcache),
    .mem_0_ar_bits_prot(m_arprot),
    .mem_0_ar_bits_qos(m_arqos),
    .mem_0_r_ready(m_rready),
    .mem_0_r_valid(m_rvalid),
    .mem_0_r_bits_id(m_rid),
    .mem_0_r_bits_data(m_rdata),
    .mem_0_r_bits_resp(m_rresp),
    .mem_0_r_bits_last(m_rlast)
  );
  assign c_b = c_b_resp;
  assign c_r = {c_r_word, c_r_resp};
  assign s_b = {s_b_id, s_b_resp};
  assign s_r = {s_r_id, s_r_word, s_r_resp, s_r_last};

  // mem_0 across to the MIG's user clock.
  wire u_awvalid, u_awready, u_wvalid, u_wready, u_bvalid, u_bready;
  wire u_arvalid, u_arready, u_rvalid, u_rready, u_wlast, u_rlast;
  wire [15:0] u_awid, u_arid, u_bid, u_rid;
  wire [29:0] u_awaddr, u_araddr;
  wire [7:0] u_awlen, u_arlen, u_wstrb;
  wire [2:0] u_awsize, u_arsize, u_awprot, u_arprot;
  wire [1:0] u_awburst, u_arburst, u_bresp, u_rresp;
  wire [0:0] u_awlock, u_arlock;
  wire [3:0] u_awcache, u_arcache, u_awqos, u_arqos;
  wire [63:0] u_wdata, u_rdata;
  rocket_mem_cc mem_cc (
    .s_axi_aclk(clk50),
    .s_axi_aresetn(~sim_rst),
    .s_axi_awid(m_awid),
    .s_axi_awaddr(m_awaddr),
    .s_axi_awlen(m_awlen),
    .s_axi_awsize(m_awsize),
    .s_axi_awburst(m_awburst),
    .s_axi_awlock(m_awlock),
    .s_axi_awcache(m_awcache),
    .s_axi_awprot(m_awprot),
    .s_axi_awregion(4'd0),
    .s_axi_awqos(m_awqos),
    .s_axi_awvalid(m_awvalid),
    .s_axi_awready(m_awready),
    .s_axi_wdata(m_wdata),
    .s_axi_wstrb(m_wstrb),
    .s_axi_wlast(m_wlast),
    .s_axi_wvalid(m_wvalid),
    .s_axi_wready(m_wready),
    .s_axi_bid(m_bid),
    .s_axi_bresp(m_bresp),
    .s_axi_bvalid(m_bvalid),
    .s_axi_bready(m_bready),
    .s_axi_arid(m_arid),
    .s_axi_araddr(m_araddr),
    .s_axi_arlen(m_arlen),
    .s_axi_arsize(m_arsize),
    .s_axi_arburst(m_arburst),
    .s_axi_arlock(m_arlock),
    .s_axi_arcache(m_arcache),
    .s_axi_arprot(m_arprot),
    .s_axi_arregion(4'd0),
    .s_axi_arqos(m_arqos),
    .s_axi_arvalid(m_arvalid),
    .s_axi_arready(m_arready),
    .s_axi_rid(m_rid),
    .s_axi_rdata(m_rdata),
    .s_axi_rresp(m_rresp),
    .s_axi_rlast(m_rlast),
    .s_axi_rvalid(m_rvalid),
    .s_axi_rready(m_rready),
    .m_axi_aclk(ui_clk),
    .m_axi_aresetn(ui_aresetn),
    .m_axi_awid(u_awid),
    .m_axi_awaddr(u_awaddr),
    .m_axi_awlen(u_awlen),
    .m_axi_awsize(u_awsize),
    .m_axi_awburst(u_awburst),
    .m_axi_awlock(u_awlock),
    .m_axi_awcache(u_awcache),
    .m_axi_awprot(u_awprot),
    .m_axi_awregion(),
    .m_axi_awqos(u_awqos),
    .m_axi_awvalid(u_awvalid),
    .m_axi_awready(u_awready),
    .m_axi_wdata(u_wdata),
    .m_axi_wstrb(u_wstrb),
    .m_axi_wlast(u_wlast),
    .m_axi_wvalid(u_wvalid),
    .m_axi_wready(u_wready),
    .m_axi_bid(u_bid),
    .m_axi_bresp(u_bresp),
    .m_axi_bvalid(u_bvalid),
    .m_axi_bready(u_bready),
    .m_axi_arid(u_arid),
    .m_axi_araddr(u_araddr),
    .m_axi_arlen(u_arlen),
    .m_axi_arsize(u_arsize),
    .m_axi_arburst(u_arburst),
    .m_axi_arlock(u_arlock),
    .m_axi_arcache(u_arcache),
    .m_axi_arprot(u_arprot),
    .m_axi_arregion(),
    .m_axi_arqos(u_arqos),
    .m_axi_arvalid(u_arvalid),
    .m_axi_arready(u_arready),
    .m_axi_rid(u_rid),
    .m_axi_rdata(u_rdata),
    .m_axi_rresp(u_rresp),
    .m_axi_rlast(u_rlast),
    .m_axi_rvalid(u_rvalid),
    .m_axi_rready(u_rready)
  );

  // The memory.
  rocket_mig mig (
    .ddr3_dq(ddr3_dq_p),
    .ddr3_dqs_n(ddr3_dqs_n),
    .ddr3_dqs_p(ddr3_dqs_p),
    .ddr3_addr(ddr3_a),
    .ddr3_ba(ddr3_ba),
    .ddr3_ras_n(ddr3_ras),
    .ddr3_cas_n(ddr3_cas),
    .ddr3_we_n(ddr3_we),
    .ddr3_reset_n(ddr3_reset),
    .ddr3_ck_p(ddr3_clk_p),
    .ddr3_ck_n(ddr3_clk_n),
    .ddr3_cke(ddr3_cke),
    .ddr3_cs_n(ddr3_s0),
    .ddr3_dm(ddr3_dm),
    .ddr3_odt(ddr3_odt),
    .sys_clk_i(clk200),
    .ui_clk(ui_clk),
    .ui_clk_sync_rst(ui_rst),
    .mmcm_locked(),
    .aresetn(ui_aresetn),
    .app_sr_req(1'b0),
    .app_ref_req(1'b0),
    .app_zq_req(1'b0),
    .app_sr_active(),
    .app_ref_ack(),
    .app_zq_ack(),
    .s_axi_awid(u_awid),
    .s_axi_awaddr(u_awaddr),
    .s_axi_awlen(u_awlen),
    .s_axi_awsize(u_awsize),
    .s_axi_awburst(u_awburst),
    .s_axi_awlock(u_awlock),
    .s_axi_awcache(u_awcache),
    .s_axi_awprot(u_awprot),
    .s_axi_awqos(u_awqos),
    .s_axi_awvalid(u_awvalid),
    .s_axi_awready(u_awready),
    .s_axi_wdata(u_wdata),
    .s_axi_wstrb(u_wstrb),
    .s_axi_wlast(u_wlast),
    .s_axi_wvalid(u_wvalid),
    .s_axi_wready(u_wready),
    .s_axi_bready(u_bready),
    .s_axi_bid(u_bid),
    .s_axi_bresp(u_bresp),
    .s_axi_bvalid(u_bvalid),
    .s_axi_arid(u_arid),
    .s_axi_araddr(u_araddr),
    .s_axi_arlen(u_arlen),
    .s_axi_arsize(u_arsize),
    .s_axi_arburst(u_arburst),
    .s_axi_arlock(u_arlock),
    .s_axi_arcache(u_arcache),
    .s_axi_arprot(u_arprot),
    .s_axi_arqos(u_arqos),
    .s_axi_arvalid(u_arvalid),
    .s_axi_arready(u_arready),
    .s_axi_rready(u_rready),
    .s_axi_rid(u_rid),
    .s_axi_rdata(u_rdata),
    .s_axi_rresp(u_rresp),
    .s_axi_rlast(u_rlast),
    .s_axi_rvalid(u_rvalid),
    .init_calib_complete(calib),
    .device_temp_i(12'd0),
    .device_temp(),
    .sys_rst(mig_sys_rst)
  );

  // Answers out: across to 125 MHz and onto the PHY.
  wire [8:0] tx_data;
  wire tx_valid, tx_ready;
  chan_cdc #(.W(9), .AW(4)) tx_cross (
    .wr_rst(sim_rst), .rd_rst(tx_rst),
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

  // The lights: locked, the memory calibrated, an answer sent.
  reg [24:0] seen_tx = 0;
  always @(posedge clk125)
    seen_tx <= tx_en ? ~25'd0 : (seen_tx != 0 ? seen_tx - 1 : 0);
  assign led1 = !locked;
  assign led2 = !calib;
  assign led3 = seen_tx == 0;
endmodule
