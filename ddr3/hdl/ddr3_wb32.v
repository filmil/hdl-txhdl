// SPDX-License-Identifier: Apache-2.0
//
// AMD's MIG 7 Series controller, `ddr3_mig`, with a Wishbone of
// thirty-two bit words in front of it: the foreign module `ddr3::Ddr3`
// names.
//
// The controller's user interface moves a burst of eight words at a
// time, 256 bits of data and 32 mask bits, which is wider than a TxHDL
// value can be and wider than a bus of words needs. This module narrows
// it and does nothing else. A word address is the controller's address
// above three bits of lane: the request goes to the controller at that
// address with its low three bits clear, the word repeated across the
// burst and the mask clear on that one word alone, so a write touches
// only its word; a read keeps its lane until the burst comes back and
// picks its word out of it. One request is in flight at a time, which
// is what a bus of words asks of it, and a write is acknowledged the
// cycle after the controller takes it.
//
// The controller makes the clocks. It takes the board's 200 MHz on
// `i_sys_clk`, which is also its delay reference, runs the memory at
// 400 MHz, and hands out `o_ui_clk`, the 100 MHz the rest of the design
// runs on, with `o_ui_rst` high until that clock is good. The design's
// clock comes back in on `i_ui_clk`; it is the same clock. The
// controller's own reset, `i_sys_rst`, is high at power-on and nothing
// else, so the memory runs through a press of the button (issue 333).
// Calibration is the controller's, in hardware; `o_calib_complete` says
// it is done. The simulation library carries the controller's variant
// with the fast calibration, so the simulation needs no parameter.
`timescale 1ps / 1ps
module ddr3_wb32 (
  input i_ui_clk,
  input i_sys_clk,
  input i_sys_rst,
  input i_wb_cyc,
  input i_wb_stb,
  input i_wb_we,
  input [27:0] i_wb_addr,
  input [31:0] i_wb_data,
  input [3:0] i_wb_sel,
  output o_wb_stall,
  output reg o_wb_ack,
  output reg [31:0] o_wb_data,
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
  wire ui_clk, ui_rst, calib, app_rdy, app_wdf_rdy, rd_valid;
  wire [255:0] rd_data;

  // The lane of the word within its burst, and the mask with that
  // word's bytes alone clear: a set bit is a byte the controller does
  // not write.
  wire [2:0] lane = i_wb_addr[2:0];
  wire [31:0] mask = ~({28'd0, i_wb_sel} << (4 * lane));

  // One request in flight. `busy` holds from a read's acceptance to its
  // data, and for the one cycle of a write's acknowledge; the stall
  // holds everything until the memory has calibrated and the controller
  // has room for both the command and the data.
  reg busy = 0, reading = 0;
  reg [2:0] lane_q = 0;
  assign o_wb_stall = busy | ~calib | ~app_rdy | ~app_wdf_rdy;
  wire take = i_wb_cyc & i_wb_stb & ~o_wb_stall;
  always @(posedge i_ui_clk) begin
    o_wb_ack <= 0;
    if (ui_rst) begin
      busy <= 0;
      reading <= 0;
    end else if (take) begin
      busy <= 1;
      reading <= ~i_wb_we;
      lane_q <= lane;
    end else if (busy & ~reading) begin
      // The write went in with its command; say so.
      busy <= 0;
      o_wb_ack <= 1;
    end else if (busy & rd_valid) begin
      busy <= 0;
      reading <= 0;
      o_wb_data <= rd_data[32 * lane_q +: 32];
      o_wb_ack <= 1;
    end
  end

  ddr3_mig ctl (
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
    // The address in the controller's order, bank then row then column,
    // with the burst's three column bits clear and the one rank's bit.
    .app_addr({1'b0, i_wb_addr[27:3], 3'd0}),
    .app_cmd({2'd0, ~i_wb_we}),
    .app_en(take),
    .app_wdf_data({8{i_wb_data}}),
    .app_wdf_end(take & i_wb_we),
    .app_wdf_mask(mask),
    .app_wdf_wren(take & i_wb_we),
    .app_rd_data(rd_data),
    .app_rd_data_end(),
    .app_rd_data_valid(rd_valid),
    .app_rdy(app_rdy),
    .app_wdf_rdy(app_wdf_rdy),
    .app_sr_req(1'b0),
    .app_ref_req(1'b0),
    .app_zq_req(1'b0),
    .app_sr_active(),
    .app_ref_ack(),
    .app_zq_ack(),
    .ui_clk(ui_clk),
    .ui_clk_sync_rst(ui_rst),
    .init_calib_complete(calib),
    // No temperature monitor: the controller is told a constant, as
    // its guide says to when the XADC is not its.
    .device_temp_i(12'd0),
    .device_temp(),
    .sys_rst(i_sys_rst)
  );
  assign o_ui_clk = ui_clk;
  assign o_ui_rst = ui_rst;
  assign o_calib_complete = calib;
endmodule
