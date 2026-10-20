// SPDX-License-Identifier: Apache-2.0
// The same two joined by TxHDL's own channel, `pair_txhdl_chan`, whose
// ready is a register (#1868): the words pass, in order.
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
    for (cycle = 0; cycle < 100; cycle = cycle + 1) begin
      #5 clk = 1;
      if (cycle == 2) rst = 0;
      if (ov) begin
        if (od != passed[7:0]) begin
          $display("buffered: word %0d was %0d", passed, od);
          $fatal(1);
        end
        passed = passed + 1;
      end
      #5 clk = 0;
    end
    if (passed < 50) begin
      $display("buffered: only %0d words passed", passed);
      $fatal(1);
    end
    $display("buffered: %0d words passed, in order", passed);
    $finish;
  end
endmodule
