// SPDX-License-Identifier: Apache-2.0
// A module written by hand, for ex_blackbox to instantiate: a count
// that steps once every PERIOD cycles while enabled, shown on eight
// pins that it drives only while enabled and leaves floating
// otherwise. A parameter, a clock pin and an inout, which are the
// three things a lowered netlist has to say to reach such a module.
`timescale 1ns/1ps
module blinker #(
  parameter PERIOD = 4
) (
  input clk,
  input enable,
  output [7:0] count,
  inout [7:0] pins
);
  reg [7:0] n = 0;
  reg [7:0] tick = 0;
  assign count = n;
  assign pins = enable ? n : 8'bz;
  always @(posedge clk) begin
    if (enable) begin
      if (tick == PERIOD - 1) begin
        tick <= 0;
        n <= n + 1;
      end else begin
        tick <= tick + 1;
      end
    end
  end
endmodule
