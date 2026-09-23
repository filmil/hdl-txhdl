// SPDX-License-Identifier: Apache-2.0
//
// The Vreteno board as it goes on the Alinx AX7A200, simulated whole:
// the top with its resets, the lowered design with the DDR3 test in
// its memories, AMD's MIG controller, and two Micron models of
// the board's x16 chips on the memory's pins.
//
// The program writes words across the memory, reads them back, says
// `ddr3 ok` or `ddr3 bad` on the serial port and halts. The testbench
// reads the serial line as a terminal would, sixteen cycles of the
// 100 MHz clock a bit as the simulation netlist has it, and `ok` is the
// verdict the test's script reads: the core halted, and the last line
// it said was `ddr3 ok`.
`timescale 1ps / 1ps
module board_tb;
  reg clk200 = 0;
  always #2500 clk200 = ~clk200;
  reg reset_n = 0;
  initial #1000000 reset_n = 1;

  wire led1, led2, led3, led4, tx;
  wire [14:0] a;
  wire [2:0] ba;
  wire ras, cas, we, s0, cke, odt, mrst, ckp, ckn;
  wire [3:0] dm;
  wire [31:0] dq;
  wire [3:0] dqs, dqs_n;

  vreteno_board dut (
    .sys_clk_p(clk200), .sys_clk_n(~clk200), .reset_n(reset_n),
    // KEY1 is a second reset, low while pressed, and nobody presses it.
    .key1(1'b1),
    .led1(led1), .led2(led2), .led3(led3), .led4(led4),
    .uart_tx(tx), .uart_rx(1'b1),
    .ddr3_a(a), .ddr3_ba(ba), .ddr3_ras(ras), .ddr3_cas(cas),
    .ddr3_we(we), .ddr3_s0(s0), .ddr3_cke(cke), .ddr3_odt(odt),
    .ddr3_reset(mrst), .ddr3_clk_p(ckp), .ddr3_clk_n(ckn),
    .ddr3_dm(dm), .ddr3_dq_p(dq), .ddr3_dqs_p(dqs), .ddr3_dqs_n(dqs_n)
  );

  ddr3_model m0 (.rst_n(mrst), .ck(ckp), .ck_n(ckn), .cke(cke), .cs_n(s0),
    .ras_n(ras), .cas_n(cas), .we_n(we), .dm_tdqs(dm[1:0]), .ba(ba),
    .addr(a), .dq(dq[15:0]), .dqs(dqs[1:0]), .dqs_n(dqs_n[1:0]),
    .tdqs_n(), .odt(odt));
  ddr3_model m1 (.rst_n(mrst), .ck(ckp), .ck_n(ckn), .cke(cke), .cs_n(s0),
    .ras_n(ras), .cas_n(cas), .we_n(we), .dm_tdqs(dm[3:2]), .ba(ba),
    .addr(a), .dq(dq[31:16]), .dqs(dqs[3:2]), .dqs_n(dqs_n[3:2]),
    .tdqs_n(), .odt(odt));

  // The terminal: a frame is a low start bit, eight bits least
  // significant first, and a high stop bit, each 160 ns long. The last
  // eight characters are kept, which is as long as `ddr3 ok\n` is.
  localparam BIT = 160000;
  reg [63:0] last = 0;
  reg [7:0] ch;
  integer i;
  initial begin
    // The line means nothing until the memory has calibrated, since the
    // design is held in reset until then, and a frame starts from the
    // idle high.
    wait (led3 == 0);
    wait (tx == 1);
    forever begin
      @(negedge tx);
      #(BIT + BIT / 2);
      for (i = 0; i < 8; i = i + 1) begin
        ch[i] = tx;
        #(BIT);
      end
      last = {last[55:0], ch};
      $display("%0t ps: the serial port said %h", $time, ch);
    end
  end

  reg ok = 0;
  initial begin
    // The PLL's lock is not on a light any more. It was `led4` until
    // 7e8d37c gave that light the heartbeat, and the heartbeat is bit
    // 25 of a counter at 100 MHz, which first rises after 335 ms: this
    // process waited here for a light that this simulation will never
    // see change, and every run since has been killed by the test's
    // budget rather than reaching its verdict. See issue #347.
    //
    // Nothing is lost by not checking it. The memory cannot calibrate
    // on a clock that is not there, so `led3` going low says the
    // controller's clock is good as surely as `led4` used to.
    wait (led3 == 0);
    $display("%0t ps: the memory calibrated", $time);
    wait (led1 == 0);
    $display("%0t ps: the core halted", $time);
    // The last byte is still going out when the core halts.
    #(BIT * 12);
    ok = (last == "ddr3 ok\n");
    $display("verdict %0d: the line ended %h", ok, last);
    $finish;
  end
  // The guard, for a run that is never going to finish. It is 500 us,
  // about three times a run that works: the controller's fast
  // calibration, the one its simulation variant carries, is done by
  // about 110 us, and the core has said its line and halted soon
  // after. It used to be
  // 20 ms, which at the speed this simulates is days of waiting, so a
  // run that was stuck was killed by the test's budget instead and said
  // nothing about how far it got, which is how the wait above went
  // unnoticed. What this prints is how far it did get. See issue #347.
  initial begin
    #(5.0e8);
    $display("timed out: calibrated %0d halted %0d, the line said %h",
      !led3, !led1, last);
    $finish;
  end
endmodule
