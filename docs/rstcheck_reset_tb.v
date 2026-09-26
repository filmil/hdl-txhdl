// SPDX-License-Identifier: Apache-2.0
// The decade counter of ex_rstcheck held in its own reset for three
// cycles, then counting past a wrap (issue 633). The checks are the
// netlist's own, compiled in with FORMAL; Verilator's --assert makes a
// failing one stop the run. Under reset the counter shows a blank,
// which its check would reject if the netlist stated it there.
`timescale 1ns/1ps
module rstcheck_reset_tb;
  reg clk = 0;
  reg rst = 1;
  reg tick = 1;
  wire [7:0] shown;
  decade uut (.clk(clk), .rst(rst), .tick(tick), .shown(shown));
  always #1 clk = ~clk;
  initial begin
    repeat (3) @(posedge clk);
    #0.5 rst = 0;
    repeat (25) @(posedge clk);
    #0.5;
    if (shown > 9) $fatal(1, "a blank shown out of reset");
    $display("no check failed across the reset");
    $finish;
  end
endmodule
