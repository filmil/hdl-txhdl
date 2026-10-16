// SPDX-License-Identifier: Apache-2.0
//
// AMD's MIG 7 Series controller generated with its AXI4 port,
// `ddr3_mig_axi`, as the foreign module `ddr3::Ddr3` names.
//
// The controller's AXI4 port is 256 bits wide, the width of its native
// interface, with narrow bursts on (issue 1696). It cuts a burst into
// the memory's commands, masks a narrow beat to its bytes, and keeps
// transactions outstanding by identifier. This module puts two ports on
// it: the thirty-two bit one, whose beats it steers to their lanes, and
// a read-only one of 256 bits beside it, the two sharing the read
// addresses; how is said below, where it is done. Otherwise it gives
// the controller the inputs a design has no reason to drive. Its AXI
// reset is the user clock's reset, inverted
// and registered, as AMD's example design makes it. The refresh,
// self-refresh and ZQ requests are never made, since the controller
// makes them itself. The temperature is a constant, as the controller's
// guide says to give it when the XADC is not its.
//
// The link's address is thirty-two bits and the port's thirty, the
// memory's whole gigabyte, so the top two bits are dropped here: the
// design decodes them before a request reaches this module, and the
// memory sees its offset.
//
// The controller makes the clocks. It takes the board's 200 MHz on
// `i_sys_clk`, which is also its delay reference, runs the memory at
// 400 MHz, and hands out `o_ui_clk`, the 100 MHz the rest of the design
// runs on, with `o_ui_rst` high until that clock is good. The design's
// clock comes back in on `i_ui_clk`; it is the same clock, and the AXI
// port is on it. The controller's own reset, `i_sys_rst`, is high at
// power-on and nothing else, so the memory runs through a press of the
// button (issue 333). Calibration is the controller's, in hardware;
// `o_calib_complete` says it is done. The simulation library carries
// the controller's variant with the fast calibration, so the simulation
// needs no parameter.
`timescale 1ps / 1ps
module ddr3_axi32 (
  input i_ui_clk,
  input i_sys_clk,
  input i_sys_rst,
  input [4:0] i_awid,
  input [31:0] i_awaddr,
  input [7:0] i_awlen,
  input [2:0] i_awsize,
  input [1:0] i_awburst,
  input i_awlock,
  input [3:0] i_awcache,
  input [2:0] i_awprot,
  input [3:0] i_awqos,
  input i_awvalid,
  input [31:0] i_wdata,
  input [3:0] i_wstrb,
  input i_wlast,
  input i_wvalid,
  input i_bready,
  input [4:0] i_arid,
  input [31:0] i_araddr,
  input [7:0] i_arlen,
  input [2:0] i_arsize,
  input [1:0] i_arburst,
  input i_arlock,
  input [3:0] i_arcache,
  input [2:0] i_arprot,
  input [3:0] i_arqos,
  input i_arvalid,
  input i_rready,
  input [4:0] i_xarid,
  input [31:0] i_xaraddr,
  input [7:0] i_xarlen,
  input i_xarvalid,
  input i_xrready,
  output o_awready,
  output o_wready,
  output [4:0] o_bid,
  output [1:0] o_bresp,
  output o_bvalid,
  output o_arready,
  output [4:0] o_rid,
  output [31:0] o_rdata,
  output [1:0] o_rresp,
  output o_rlast,
  output o_rvalid,
  output o_xarready,
  output [4:0] o_xrid,
  output [255:0] o_xrdata,
  output [1:0] o_xrresp,
  output o_xrlast,
  output o_xrvalid,
  output o_calib_complete,
  output o_ui_clk,
  output o_ui_rst,
  output o_ddr3_clk_p,
  output o_ddr3_clk_n,
  output o_ddr3_reset_n,
  output o_ddr3_cke,
  output o_ddr3_cs_n,
  output o_ddr3_ras_n,
  output o_ddr3_cas_n,
  output o_ddr3_we_n,
  output [14:0] o_ddr3_addr,
  output [2:0] o_ddr3_ba_addr,
  output [3:0] o_ddr3_dm,
  output o_ddr3_odt,
  inout [31:0] io_ddr3_dq,
  inout [3:0] io_ddr3_dqs,
  inout [3:0] io_ddr3_dqs_n
);
  wire ui_clk, ui_rst, calib;

  // The AXI reset, low while the user clock's reset is high.
  reg aresetn = 0;
  always @(posedge i_ui_clk) aresetn <= ~ui_rst;


  // The controller's port is 256 bits, with narrow bursts on (issue
  // 1696). The thirty-two bit side's beats stay four-byte transfers:
  // a write's word is repeated across the lanes and its strobe moved to
  // the beat's lane, and a read beat is the word at its lane. The
  // identifier gains a top bit, the side: 0 for the thirty-two bit
  // side, which alone writes, and 1 for the wide read port.

  // Writes. Each burst's first lane, in address order, for its beats;
  // the beats of one burst follow its address phase, as AXI4 asks.
  wire m_awready, m_wready;
  wire [5:0] m_bid;
  reg [2:0] wq [0:7];
  reg [3:0] wq_head = 0, wq_tail = 0;
  reg [2:0] wk = 0;
  wire wq_full = (wq_tail - wq_head) == 4'd8;
  wire wq_empty = wq_tail == wq_head;
  wire m_awvalid = i_awvalid & ~wq_full;
  assign o_awready = m_awready & ~wq_full;
  wire m_wvalid = i_wvalid & ~wq_empty;
  assign o_wready = m_wready & ~wq_empty;
  wire [2:0] wlane = wq[wq_head[2:0]] + wk;
  wire [31:0] m_wstrb = {28'd0, i_wstrb} << {wlane, 2'b00};
  always @(posedge i_ui_clk) begin
    if (m_awvalid & m_awready) begin
      wq[wq_tail[2:0]] <= i_awaddr[4:2];
      wq_tail <= wq_tail + 4'd1;
    end
    if (m_wvalid & m_wready) begin
      if (i_wlast) begin
        wq_head <= wq_head + 4'd1;
        wk <= 3'd0;
      end else
        wk <= wk + 3'd1;
    end
  end
  assign o_bid = m_bid[4:0];

  // Read addresses: the two sides in turn, a grant held from the step
  // an address is offered until it is taken, so what the controller
  // sees offered does not change under it.
  wire m_arready;
  reg held = 0, held_x = 0, last_x = 0;
  wire pick_x = held ? held_x
              : (i_xarvalid & (~i_arvalid | ~last_x));
  wire m_arvalid = pick_x ? i_xarvalid : i_arvalid;
  wire [5:0] m_arid = pick_x ? {1'b1, i_xarid} : {1'b0, i_arid};
  wire [29:0] m_araddr = pick_x ? i_xaraddr[29:0] : i_araddr[29:0];
  wire [7:0] m_arlen = pick_x ? i_xarlen : i_arlen;
  wire [2:0] m_arsize = pick_x ? 3'd5 : i_arsize;
  wire [1:0] m_arburst = pick_x ? 2'b01 : i_arburst;
  wire m_arlock = pick_x ? 1'b0 : i_arlock;
  wire [3:0] m_arcache = pick_x ? 4'd0 : i_arcache;
  wire [2:0] m_arprot = pick_x ? 3'd0 : i_arprot;
  wire [3:0] m_arqos = pick_x ? 4'd0 : i_arqos;
  assign o_arready = m_arready & ~pick_x;
  assign o_xarready = m_arready & pick_x;
  always @(posedge i_ui_clk) begin
    held <= m_arvalid & ~m_arready;
    held_x <= pick_x;
    if (m_arvalid & m_arready) last_x <= pick_x;
  end

  // Read beats, back to the side their identifier names; a thirty-two
  // bit beat is the word at its lane, which moves on a lane a beat from
  // the one the burst's address named, kept per identifier.
  wire [5:0] m_rid;
  wire [255:0] m_rdata;
  wire [1:0] m_rresp;
  wire m_rlast, m_rvalid;
  reg [2:0] rlane [0:31];
  wire to_x = m_rid[5];
  wire [2:0] lane = rlane[m_rid[4:0]];
  wire m_rready = to_x ? i_xrready : i_rready;
  assign o_rvalid = m_rvalid & ~to_x;
  assign o_rid = m_rid[4:0];
  assign o_rdata = m_rdata[{lane, 5'd0} +: 32];
  assign o_rresp = m_rresp;
  assign o_rlast = m_rlast;
  assign o_xrvalid = m_rvalid & to_x;
  assign o_xrid = m_rid[4:0];
  assign o_xrdata = m_rdata;
  assign o_xrresp = m_rresp;
  assign o_xrlast = m_rlast;
  always @(posedge i_ui_clk) begin
    if (o_arready & i_arvalid)
      rlane[i_arid] <= i_araddr[4:2];
    if (o_rvalid & i_rready & (o_arready & i_arvalid ? i_arid != m_rid[4:0] : 1'b1))
      rlane[m_rid[4:0]] <= lane + 3'd1;
  end

  ddr3_mig_axi ctl (
    .ddr3_dq(io_ddr3_dq),
    .ddr3_dqs_n(io_ddr3_dqs_n),
    .ddr3_dqs_p(io_ddr3_dqs),
    .ddr3_addr(o_ddr3_addr),
    .ddr3_ba(o_ddr3_ba_addr),
    .ddr3_ras_n(o_ddr3_ras_n),
    .ddr3_cas_n(o_ddr3_cas_n),
    .ddr3_we_n(o_ddr3_we_n),
    .ddr3_reset_n(o_ddr3_reset_n),
    .ddr3_ck_p(o_ddr3_clk_p),
    .ddr3_ck_n(o_ddr3_clk_n),
    .ddr3_cke(o_ddr3_cke),
    .ddr3_cs_n(o_ddr3_cs_n),
    .ddr3_dm(o_ddr3_dm),
    .ddr3_odt(o_ddr3_odt),
    .sys_clk_i(i_sys_clk),
    .ui_clk(ui_clk),
    .ui_clk_sync_rst(ui_rst),
    .mmcm_locked(),
    .aresetn(aresetn),
    .app_sr_req(1'b0),
    .app_ref_req(1'b0),
    .app_zq_req(1'b0),
    .app_sr_active(),
    .app_ref_ack(),
    .app_zq_ack(),
    .s_axi_awid({1'b0, i_awid}),
    .s_axi_awaddr(i_awaddr[29:0]),
    .s_axi_awlen(i_awlen),
    .s_axi_awsize(i_awsize),
    .s_axi_awburst(i_awburst),
    .s_axi_awlock(i_awlock),
    .s_axi_awcache(i_awcache),
    .s_axi_awprot(i_awprot),
    .s_axi_awqos(i_awqos),
    .s_axi_awvalid(m_awvalid),
    .s_axi_awready(m_awready),
    .s_axi_wdata({8{i_wdata}}),
    .s_axi_wstrb(m_wstrb),
    .s_axi_wlast(i_wlast),
    .s_axi_wvalid(m_wvalid),
    .s_axi_wready(m_wready),
    .s_axi_bready(i_bready),
    .s_axi_bid(m_bid),
    .s_axi_bresp(o_bresp),
    .s_axi_bvalid(o_bvalid),
    .s_axi_arid(m_arid),
    .s_axi_araddr(m_araddr),
    .s_axi_arlen(m_arlen),
    .s_axi_arsize(m_arsize),
    .s_axi_arburst(m_arburst),
    .s_axi_arlock(m_arlock),
    .s_axi_arcache(m_arcache),
    .s_axi_arprot(m_arprot),
    .s_axi_arqos(m_arqos),
    .s_axi_arvalid(m_arvalid),
    .s_axi_arready(m_arready),
    .s_axi_rready(m_rready),
    .s_axi_rid(m_rid),
    .s_axi_rdata(m_rdata),
    .s_axi_rresp(m_rresp),
    .s_axi_rlast(m_rlast),
    .s_axi_rvalid(m_rvalid),
    .init_calib_complete(calib),
    .device_temp_i(12'd0),
    .device_temp(),
    .sys_rst(i_sys_rst)
  );
  assign o_ui_clk = ui_clk;
  assign o_ui_rst = ui_rst;
  assign o_calib_complete = calib;
endmodule
