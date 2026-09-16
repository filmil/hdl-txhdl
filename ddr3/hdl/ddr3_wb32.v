// SPDX-License-Identifier: Apache-2.0
//
// UberDDR3's controller, `ddr3_top`, with a Wishbone of thirty-two bit
// words in front of it: the foreign module `ddr3::Ddr3` names.
//
// The controller's own Wishbone carries a burst of eight words per
// address, 256 bits of data and 32 lane bits, which is wider than a
// TxHDL value can be and wider than a bus of words needs. This module
// narrows it and does nothing else. A word address is the controller's
// address above three bits of lane: the request goes to the controller
// at the address, with the word repeated across the burst and the lane
// bits of that one word alone set, so a write touches only its word;
// and the lane rides on the controller's `aux` lines, which it hands
// back with the acknowledge, so the word read is picked out of the
// burst by the lane of the request it answers. No register, no state:
// the module is the controller and wires.
//
// The parameters are the controller's, with the defaults the a200t
// examples run on the Alinx AX7A200: a 100 MHz controller clock and a
// 400 MHz memory clock, x16 chips on four lanes, DDR3-1066 timing.
`timescale 1ps / 1ps
module ddr3_wb32 #(
  parameter CONTROLLER_CLK_PERIOD = 10000,
  parameter DDR3_CLK_PERIOD = 2500,
  parameter ROW_BITS = 15,
  parameter COL_BITS = 10,
  parameter BA_BITS = 3,
  parameter BYTE_LANES = 4,
  parameter SPEED_BIN = 1,
  parameter SDRAM_CAPACITY = 5,
  parameter DLL_OFF = 0,
  parameter MICRON_SIM = 0,
  parameter BIST_MODE = 1
) (
  input i_controller_clk,
  input i_ddr3_clk,
  input i_ref_clk,
  input i_ddr3_clk_90,
  input i_rst_n,
  input i_wb_cyc,
  input i_wb_stb,
  input i_wb_we,
  input [ROW_BITS+COL_BITS+BA_BITS-1:0] i_wb_addr,
  input [31:0] i_wb_data,
  input [3:0] i_wb_sel,
  output o_wb_stall,
  output o_wb_ack,
  output [31:0] o_wb_data,
  output o_calib_complete,
  output o_ddr3_clk_p,
  output o_ddr3_clk_n,
  output o_ddr3_reset_n,
  output o_ddr3_cke,
  output o_ddr3_cs_n,
  output o_ddr3_ras_n,
  output o_ddr3_cas_n,
  output o_ddr3_we_n,
  output [ROW_BITS-1:0] o_ddr3_addr,
  output [BA_BITS-1:0] o_ddr3_ba_addr,
  output [BYTE_LANES-1:0] o_ddr3_dm,
  output o_ddr3_odt,
  inout [8*BYTE_LANES-1:0] io_ddr3_dq,
  inout [BYTE_LANES-1:0] io_ddr3_dqs,
  inout [BYTE_LANES-1:0] io_ddr3_dqs_n
);
  // The lane of the word within its burst, and the lane bits of that
  // word alone.
  wire [2:0] lane = i_wb_addr[2:0];
  wire [31:0] lanes = {28'd0, i_wb_sel} << (4 * lane);
  wire [3:0] aux_back;
  wire [255:0] burst;

  ddr3_top #(
    .CONTROLLER_CLK_PERIOD(CONTROLLER_CLK_PERIOD),
    .DDR3_CLK_PERIOD(DDR3_CLK_PERIOD),
    .ROW_BITS(ROW_BITS),
    .COL_BITS(COL_BITS),
    .BA_BITS(BA_BITS),
    .BYTE_LANES(BYTE_LANES),
    .AUX_WIDTH(4),
    .SPEED_BIN(SPEED_BIN),
    .SDRAM_CAPACITY(SDRAM_CAPACITY),
    .DLL_OFF(DLL_OFF),
    .MICRON_SIM(MICRON_SIM),
    .BIST_MODE(BIST_MODE)
  ) ctl (
    .i_controller_clk(i_controller_clk),
    .i_ddr3_clk(i_ddr3_clk),
    .i_ref_clk(i_ref_clk),
    .i_ddr3_clk_90(i_ddr3_clk_90),
    .i_rst_n(i_rst_n),
    .i_wb_cyc(i_wb_cyc),
    .i_wb_stb(i_wb_stb),
    .i_wb_we(i_wb_we),
    .i_wb_addr(i_wb_addr[ROW_BITS+COL_BITS+BA_BITS-1:3]),
    .i_wb_data({8{i_wb_data}}),
    .i_wb_sel(lanes),
    .i_aux({1'b0, lane}),
    .o_wb_stall(o_wb_stall),
    .o_wb_ack(o_wb_ack),
    .o_wb_err(),
    .o_wb_data(burst),
    .o_aux(aux_back),
    .i_wb2_cyc(1'b0),
    .i_wb2_stb(1'b0),
    .i_wb2_we(1'b0),
    .i_wb2_addr(7'd0),
    .i_wb2_data(32'd0),
    .i_wb2_sel(4'd0),
    .o_wb2_stall(),
    .o_wb2_ack(),
    .o_wb2_data(),
    .o_ddr3_clk_p(o_ddr3_clk_p),
    .o_ddr3_clk_n(o_ddr3_clk_n),
    .o_ddr3_reset_n(o_ddr3_reset_n),
    .o_ddr3_cke(o_ddr3_cke),
    .o_ddr3_cs_n(o_ddr3_cs_n),
    .o_ddr3_ras_n(o_ddr3_ras_n),
    .o_ddr3_cas_n(o_ddr3_cas_n),
    .o_ddr3_we_n(o_ddr3_we_n),
    .o_ddr3_addr(o_ddr3_addr),
    .o_ddr3_ba_addr(o_ddr3_ba_addr),
    .io_ddr3_dq(io_ddr3_dq),
    .io_ddr3_dqs(io_ddr3_dqs),
    .io_ddr3_dqs_n(io_ddr3_dqs_n),
    .o_ddr3_dm(o_ddr3_dm),
    .o_ddr3_odt(o_ddr3_odt),
    .o_calib_complete(o_calib_complete),
    .o_debug1(),
    .i_user_self_refresh(1'b0),
    .uart_tx()
  );

  // The word the acknowledge answers, by the lane it came back with.
  assign o_wb_data = burst[32 * aux_back[2:0] +: 32];
endmodule
