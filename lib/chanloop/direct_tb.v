// SPDX-License-Identifier: Apache-2.0
// `count` joined to `stage` wire to wire, as a hand-written top joins
// two netlists (#1868). `count`'s valid is its ready, and `stage`'s
// ready is its valid with room downstream, so the two wires are a loop
// with nothing driving it: low and high are both consistent, and from
// reset, where nothing has raised either, it stays low and no word
// passes. Verilator says as much, flagging the loop as circular logic,
// which is why this test alone builds with -Wno-fatal. The test passes
// when no word passes, which is the deadlock.
module direct_tb;
  reg clk = 0;
  reg rst = 1;
  wire [7:0] d, od;
  wire v, r, ov;
  count u_count(.clk(clk), .rst(rst), .out_data(d), .out_valid(v),
                .out_ready(r));
  stage u_stage(.clk(clk), .rst(rst), .inp_data(d), .inp_valid(v),
                .inp_ready(r), .out_data(od), .out_valid(ov),
                .out_ready(1'b1));
  integer passed = 0;
  integer cycle;
  initial begin
    // The clock rises at 5, 15, ...; reset is let go and the outputs
    // are looked at on the falling edge, away from the rising one.
    for (cycle = 0; cycle < 100; cycle = cycle + 1) begin
      #5 clk = 1;
      #5 clk = 0;
      if (cycle == 2) rst = 0;
      if (ov) passed = passed + 1;
    end
    if (passed != 0) begin
      $display("direct: %0d words passed; the join did not deadlock", passed);
      $fatal(1);
    end
    $display("direct: no word passed in 100 cycles: the deadlock of #1868");
    $finish;
  end
endmodule
