// SPDX-License-Identifier: Apache-2.0
//
// The controller as the board will run it, against the memory it will
// run with: `ddr3_axi32`, AMD's MIG controller with its AXI4 port
// behind it, and two Micron models of the x16 chips the Alinx AX7A200B
// carries, on the board's 200 MHz clock. The controller makes the
// design's clock, and the AXI side of this bench runs on what it hands
// out.
//
// It waits for calibration, then writes through the AXI4 port and reads
// back: a burst of sixteen words, a word alone in another row, a word
// through an address with its top two bits set, which the wrapper drops,
// a write of half a word under its strobes, and a byte written and read
// at an address that is not a word's, as the core's byte accesses are. It also times the port,
// in cycles of the user clock: from a read's address phase taken to its
// first beat, and from a write's last beat taken to its response. Those
// are the latencies `ddr3::Ddr3`'s model answers with, less the one
// cycle the model takes to see what the pins part drove.
//
// `ok` is the verdict the test's script reads when the run is done; a
// word that came back wrong, a response that was not OKAY, or a
// calibration that never finished before the time limit, leaves it low.
`timescale 1ps / 1ps
module wrapper_tb;
  reg sys_clk = 1;
  always #2500 sys_clk = ~sys_clk;
  // The controller's reset: high at power-on, as the board holds it.
  reg sys_rst = 1;
  initial #200000 sys_rst = 0;

  wire ck, ui_rst, calib;
  reg [31:0] awaddr = 0, araddr = 0, wdata = 0;
  reg [7:0] awlen = 0, arlen = 0;
  reg [3:0] wstrb = 4'hf;
  reg awvalid = 0, wvalid = 0, wlast = 0, arvalid = 0;
  wire awready, wready, bvalid, arready, rvalid, rlast;
  wire [4:0] bid, rid;
  wire [1:0] bresp, rresp;
  wire [31:0] rdata;
  wire ckp, ckn, mrst, cke, csn, rasn, casn, wen, odt;
  wire [14:0] a;
  wire [2:0] ba;
  wire [3:0] dm;
  wire [31:0] dq;
  wire [3:0] dqs, dqs_n;

  // Every burst incrementing words, normal, unprotected, with one
  // identifier, as the pins part drives what the link asks for.
  ddr3_axi32 dut (
    .i_ui_clk(ck), .i_sys_clk(sys_clk), .i_sys_rst(sys_rst),
    .i_awid(5'd3), .i_awaddr(awaddr), .i_awlen(awlen), .i_awsize(3'd2),
    .i_awburst(2'b01), .i_awlock(1'b0), .i_awcache(4'b0011),
    .i_awprot(3'd0), .i_awqos(4'd0), .i_awvalid(awvalid),
    .i_wdata(wdata), .i_wstrb(wstrb), .i_wlast(wlast), .i_wvalid(wvalid),
    .i_bready(1'b1),
    .i_arid(5'd7), .i_araddr(araddr), .i_arlen(arlen), .i_arsize(3'd2),
    .i_arburst(2'b01), .i_arlock(1'b0), .i_arcache(4'b0011),
    .i_arprot(3'd0), .i_arqos(4'd0), .i_arvalid(arvalid),
    .i_rready(1'b1),
    .o_awready(awready), .o_wready(wready), .o_bid(bid), .o_bresp(bresp),
    .o_bvalid(bvalid), .o_arready(arready), .o_rid(rid), .o_rdata(rdata),
    .o_rresp(rresp), .o_rlast(rlast), .o_rvalid(rvalid),
    .o_calib_complete(calib), .o_ui_clk(ck), .o_ui_rst(ui_rst),
    .o_ddr3_clk_p(ckp), .o_ddr3_clk_n(ckn), .o_ddr3_reset_n(mrst),
    .o_ddr3_cke(cke), .o_ddr3_cs_n(csn), .o_ddr3_ras_n(rasn),
    .o_ddr3_cas_n(casn), .o_ddr3_we_n(wen), .o_ddr3_addr(a),
    .o_ddr3_ba_addr(ba), .o_ddr3_dm(dm), .o_ddr3_odt(odt),
    .io_ddr3_dq(dq), .io_ddr3_dqs(dqs), .io_ddr3_dqs_n(dqs_n)
  );

  // Two x16 chips, a pair of lanes each.
  ddr3_model m0 (.rst_n(mrst), .ck(ckp), .ck_n(ckn), .cke(cke),
    .cs_n(csn), .ras_n(rasn), .cas_n(casn), .we_n(wen),
    .dm_tdqs(dm[1:0]), .ba(ba), .addr(a), .dq(dq[15:0]), .dqs(dqs[1:0]),
    .dqs_n(dqs_n[1:0]), .tdqs_n(), .odt(odt));
  ddr3_model m1 (.rst_n(mrst), .ck(ckp), .ck_n(ckn), .cke(cke),
    .cs_n(csn), .ras_n(rasn), .cas_n(casn), .we_n(wen),
    .dm_tdqs(dm[3:2]), .ba(ba), .addr(a), .dq(dq[31:16]), .dqs(dqs[3:2]),
    .dqs_n(dqs_n[3:2]), .tdqs_n(), .odt(odt));

  // Cycles of the user clock, read where a handshake is seen.
  integer cyc = 0;
  always @(posedge ck) cyc <= cyc + 1;

  reg ok = 1;
  integer lat;

  // A write burst of `len + 1` words from `seed` up, under `strb`: the
  // address phase offered until taken, then a beat a cycle as the port
  // takes them, then the response; `lat` is the cycles from the last
  // beat taken to the response.
  task write_burst(input [31:0] at, input [7:0] len, input [31:0] seed,
                   input [3:0] strb);
    integer i, t;
    begin
      @(posedge ck); awaddr <= at; awlen <= len; awvalid <= 1;
      @(posedge ck); while (!awready) @(posedge ck);
      awvalid <= 0;
      for (i = 0; i <= len; i = i + 1) begin
        wdata <= seed + i; wstrb <= strb; wlast <= (i == len); wvalid <= 1;
        @(posedge ck); while (!wready) @(posedge ck);
      end
      t = cyc; wvalid <= 0; wlast <= 0;
      while (!bvalid) @(posedge ck);
      lat = cyc - t;
      if (bresp != 2'b00 || bid != 5'd3) ok = 0;
      $display("write %h len %0d: response after %0d cycles", at, len + 1, lat);
    end
  endtask

  // A read burst of `len + 1` words, each checked against `want`, then
  // `want` plus one, and so on; `lat` is the cycles from the address
  // phase taken to the first beat.
  task read_burst(input [31:0] at, input [7:0] len, input [31:0] want);
    integer i, t;
    begin
      @(posedge ck); araddr <= at; arlen <= len; arvalid <= 1;
      @(posedge ck); while (!arready) @(posedge ck);
      arvalid <= 0; t = cyc; i = 0; lat = -1;
      while (i <= len) begin
        if (rvalid) begin
          if (lat < 0) lat = cyc - t;
          if (rdata !== want + i || rresp != 2'b00 || rid != 5'd7
              || rlast !== (i == len)) begin
            ok = 0;
            $display("read %h beat %0d: %h, want %h", at, i, rdata, want + i);
          end
          i = i + 1;
        end
        if (i <= len) @(posedge ck);
      end
      $display("read %h len %0d: first beat after %0d cycles", at, len + 1, lat);
    end
  endtask

  initial begin
    wait (calib);
    $display("calibrated at %0t ps", $time);
    // A burst, written and read back, twice, so that the second read
    // finds its row open.
    write_burst(32'h0000_1000, 8'd15, 32'h1000_0000, 4'hf);
    read_burst(32'h0000_1000, 8'd15, 32'h1000_0000);
    read_burst(32'h0000_1000, 8'd15, 32'h1000_0000);
    // A word alone, in another row and bank.
    write_burst(32'h0123_4564, 8'd0, 32'h89ab_cdef, 4'hf);
    read_burst(32'h0123_4564, 8'd0, 32'h89ab_cdef);
    // The top two bits are the design's to decode: the word written at
    // 0x4000_2000 is the word at 0x0000_2000.
    write_burst(32'h4000_2000, 8'd0, 32'h5555_aaaa, 4'hf);
    read_burst(32'h0000_2000, 8'd0, 32'h5555_aaaa);
    // Half a word under its strobes: the low half changes, the high
    // half keeps what the burst wrote there.
    write_burst(32'h0000_1004, 8'd0, 32'hffff_beef, 4'b0011);
    read_burst(32'h0000_1004, 8'd0, 32'h1000_beef);
    // A byte at an address that is not a word's, as the core makes a
    // byte store and load: a full-width beat at the byte's address under
    // that byte's strobe, and the whole word back on a read of it.
    write_burst(32'h0000_1009, 8'd0, 32'h0000_5a00, 4'b0010);
    read_burst(32'h0000_1009, 8'd0, 32'h1000_5a02);
    $display("verdict %0d at %0t ps", ok, $time);
    $finish;
  end
  // A calibration that has not finished in five milliseconds will not.
  initial begin
    #(5.0e9);
    ok = 0;
    $display("timed out, calibrated %0d", calib);
    $finish;
  end
endmodule
