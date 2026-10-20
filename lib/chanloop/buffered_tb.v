// SPDX-License-Identifier: Apache-2.0
// The same two joined by TxHDL's own channel, `pair_txhdl_chan`, whose
// ready is a register (#1868): the words pass, a word a cycle, in
// order.
module buffered_tb;
  reg clk = 0;
  reg rst = 1;
  wire [7:0] od;
  wire ov;
  pair u_pair(.clk(clk), .rst(rst), .out_data(od), .out_valid(ov),
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
      if (ov) begin
        if (od != passed[7:0]) begin
          $display("buffered: word %0d was %0d", passed, od);
          $fatal(1);
        end
        passed = passed + 1;
      end
    end
    // A word a cycle once reset is let go, less the cycles the
    // channel takes to fill.
    if (passed < 95) begin
      $display("buffered: only %0d words passed", passed);
      $fatal(1);
    end
    $display("buffered: %0d words passed, in order", passed);
    $finish;
  end
endmodule
