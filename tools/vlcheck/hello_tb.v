// SPDX-License-Identifier: Apache-2.0
// The smallest Verilog testbench: does the hermetic Verilator run one?
module hello_tb;
  reg clk = 0;
  always #1 clk = ~clk;
  integer n = 0;
  always @(posedge clk) n <= n + 1;
  initial begin
    #10;
    if (n != 5) $fatal(1, "n is %0d, expected 5", n);
    $display("hello: n = %0d", n);
    $finish;
  end
endmodule
