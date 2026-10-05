// SPDX-License-Identifier: Apache-2.0
//
// AMD's MIG 7 Series controller generated with its AXI4 port,
// `ddr3_mig_axi`, as the foreign module `ddr3::Ddr3` names.
//
// The controller does the work itself: it cuts a burst into the
// memory's commands, packs thirty-two bit beats into its 256-bit native
// interface and masks them, and keeps transactions outstanding by
// identifier. This module only gives it the inputs a design has no
// reason to drive. Its AXI reset is the user clock's reset, inverted
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
    .s_axi_awid(i_awid),
    .s_axi_awaddr(i_awaddr[29:0]),
    .s_axi_awlen(i_awlen),
    .s_axi_awsize(i_awsize),
    .s_axi_awburst(i_awburst),
    .s_axi_awlock(i_awlock),
    .s_axi_awcache(i_awcache),
    .s_axi_awprot(i_awprot),
    .s_axi_awqos(i_awqos),
    .s_axi_awvalid(i_awvalid),
    .s_axi_awready(o_awready),
    .s_axi_wdata(i_wdata),
    .s_axi_wstrb(i_wstrb),
    .s_axi_wlast(i_wlast),
    .s_axi_wvalid(i_wvalid),
    .s_axi_wready(o_wready),
    .s_axi_bready(i_bready),
    .s_axi_bid(o_bid),
    .s_axi_bresp(o_bresp),
    .s_axi_bvalid(o_bvalid),
    .s_axi_arid(i_arid),
    .s_axi_araddr(i_araddr[29:0]),
    .s_axi_arlen(i_arlen),
    .s_axi_arsize(i_arsize),
    .s_axi_arburst(i_arburst),
    .s_axi_arlock(i_arlock),
    .s_axi_arcache(i_arcache),
    .s_axi_arprot(i_arprot),
    .s_axi_arqos(i_arqos),
    .s_axi_arvalid(i_arvalid),
    .s_axi_arready(o_arready),
    .s_axi_rready(i_rready),
    .s_axi_rid(o_rid),
    .s_axi_rdata(o_rdata),
    .s_axi_rresp(o_rresp),
    .s_axi_rlast(o_rlast),
    .s_axi_rvalid(o_rvalid),
    .init_calib_complete(calib),
    .device_temp_i(12'd0),
    .device_temp(),
    .sys_rst(i_sys_rst)
  );
  assign o_ui_clk = ui_clk;
  assign o_ui_rst = ui_rst;
  assign o_calib_complete = calib;
endmodule
