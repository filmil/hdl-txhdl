// SPDX-License-Identifier: Apache-2.0
// The endpoint and the design behind it under Vivado's simulator (issue
// 305): the board's top, `pcie_top`, with the XDMA core inside it and
// the lowered `BarRegs` behind BAR1, elaborated whole and run. The
// reference clock is driven and the reset released; the lanes have
// nothing on them, so the link never trains and `user_lnk_up` stays
// low, which is what the verdict reads. A link that trains and a root
// port that enumerates the endpoint are issue 178: this is the check
// that every source the core needs is in its library and that the
// whole of it elaborates, which until the export of the IP's
// simulation nothing could show.
`timescale 1ns/1ps
module xdma_tb;
  reg clk_p = 1'b0;
  always #5 clk_p = ~clk_p;  // 100 MHz, the slot's reference
  wire clk_n = ~clk_p;
  reg reset_n = 1'b0;
  wire [1:0] tx_p, tx_n;
  wire led1, led2, led3, led4;
  reg ok = 1'b0;

  pcie_top dut (
      .pcie_clk_p(clk_p), .pcie_clk_n(clk_n),
      .pcie_rx_p(2'b00), .pcie_rx_n(2'b11),
      .pcie_tx_p(tx_p), .pcie_tx_n(tx_n),
      .reset_n(reset_n),
      .led1(led1), .led2(led2), .led3(led3), .led4(led4)
  );

  initial begin
    #1000 reset_n = 1'b1;
    // Long enough for the core's own reset sequence and its clock
    // generator to settle, and to show the link staying down.
    #20000;
    // A known value rather than an unknown: the core is modelled,
    // and with no partner on the lanes it reports no link.
    ok = (dut.link_up === 1'b0);
    $display("xdma_tb: link_up=%b aresetn=%b ok=%b", dut.link_up,
             dut.aresetn, ok);
    $finish;
  end
endmodule
