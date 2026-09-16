// SPDX-License-Identifier: Apache-2.0
//
// The controller as the board will run it, against the memory it will
// run with: `ddr3_wb32`, UberDDR3 behind it, and two Micron models of
// the x16 chips the Alinx AX7A200 carries, on the clocks the board's
// PLL gives, 100 MHz for the controller, 400 MHz for the memory and the
// same a quarter cycle late, and 200 MHz for the delay control.
//
// It waits for calibration, then writes two words through the narrow
// Wishbone, at different addresses and different lanes of their
// bursts, and reads both back. `ok` is the verdict the test's script
// reads when the run is done; a word that came back wrong, or a
// calibration that never finished before the time limit, leaves it
// low.
`timescale 1ps / 1ps
module wrapper_tb;
  reg ck = 1, dck = 1, dck90 = 1, rck = 1;
  always #5000 ck = ~ck;
  always #1250 dck = ~dck;
  always #2500 rck = ~rck;
  initial begin #625; forever #1250 dck90 = ~dck90; end
  reg rst_n = 0;
  initial #100000 rst_n = 1;

  reg cyc = 0, stb = 0, we = 0;
  reg [27:0] adr = 0;
  reg [31:0] dat = 0;
  reg [3:0] sel = 4'hf;
  wire stall, ack, calib;
  wire [31:0] rdat;
  wire ckp, ckn, mrst, cke, csn, rasn, casn, wen, odt;
  wire [14:0] a;
  wire [2:0] ba;
  wire [3:0] dm;
  wire [31:0] dq;
  wire [3:0] dqs, dqs_n;

  ddr3_wb32 #(.MICRON_SIM(1), .BIST_MODE(0)) dut (
    .i_controller_clk(ck), .i_ddr3_clk(dck), .i_ref_clk(rck),
    .i_ddr3_clk_90(dck90), .i_rst_n(rst_n),
    .i_wb_cyc(cyc), .i_wb_stb(stb), .i_wb_we(we), .i_wb_addr(adr),
    .i_wb_data(dat), .i_wb_sel(sel),
    .o_wb_stall(stall), .o_wb_ack(ack), .o_wb_data(rdat),
    .o_calib_complete(calib),
    .o_ddr3_clk_p(ckp), .o_ddr3_clk_n(ckn), .o_ddr3_reset_n(mrst),
    .o_ddr3_cke(cke), .o_ddr3_cs_n(csn), .o_ddr3_ras_n(rasn),
    .o_ddr3_cas_n(casn), .o_ddr3_we_n(wen), .o_ddr3_addr(a),
    .o_ddr3_ba_addr(ba), .o_ddr3_dm(dm), .o_ddr3_odt(odt),
    .io_ddr3_dq(dq), .io_ddr3_dqs(dqs), .io_ddr3_dqs_n(dqs_n)
  );

  // Two x16 chips, a pair of lanes each. The model's address is sixteen
  // bits and the controller drives fifteen.
  ddr3 m0 (.rst_n(mrst), .ck(ckp), .ck_n(ckn), .cke(cke), .cs_n(csn),
    .ras_n(rasn), .cas_n(casn), .we_n(wen), .dm_tdqs(dm[1:0]), .ba(ba),
    .addr({1'b0, a}), .dq(dq[15:0]), .dqs(dqs[1:0]), .dqs_n(dqs_n[1:0]),
    .tdqs_n(), .odt(odt));
  ddr3 m1 (.rst_n(mrst), .ck(ckp), .ck_n(ckn), .cke(cke), .cs_n(csn),
    .ras_n(rasn), .cas_n(casn), .we_n(wen), .dm_tdqs(dm[3:2]), .ba(ba),
    .addr({1'b0, a}), .dq(dq[31:16]), .dqs(dqs[3:2]), .dqs_n(dqs_n[3:2]),
    .tdqs_n(), .odt(odt));

  // One request on the pipelined Wishbone: offered until a cycle with no
  // stall takes it, then waited on until the acknowledge.
  reg [31:0] q;
  task access(input w, input [27:0] at, input [31:0] v);
    begin
      @(posedge ck); cyc <= 1; stb <= 1; we <= w; adr <= at; dat <= v;
      @(posedge ck); while (stall) @(posedge ck);
      stb <= 0;
      while (!ack) @(posedge ck);
      q = rdat;
      cyc <= 0;
    end
  endtask

  reg ok = 0;
  initial begin
    wait (calib);
    $display("calibrated at %0t ps", $time);
    access(1, 28'h0000005, 32'h0123_4567);
    access(1, 28'h0100_0102, 32'h89ab_cdef);
    access(0, 28'h0000005, 32'h0);
    $display("word 0x0000005 read %h", q);
    ok = (q == 32'h0123_4567);
    access(0, 28'h0100_0102, 32'h0);
    $display("word 0x0100102 read %h", q);
    ok = ok & (q == 32'h89ab_cdef);
    $display("verdict %0d at %0t ps", ok, $time);
    $finish;
  end
  // A calibration that has not finished in five milliseconds will not.
  initial begin
    #(5.0e9);
    $display("timed out, calibrated %0d", calib);
    $finish;
  end
endmodule
