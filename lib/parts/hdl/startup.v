// SPDX-License-Identifier: Apache-2.0
//
// The configuration flash's clock pin, reached from user logic after
// the bitstream has loaded (issue 312). On a 7-series part that pin is
// CCLK, and only the STARTUPE2 primitive reaches it: `usrcclko` is the
// clock user logic drives, and `eos` says configuration is over, the
// end of startup. `Startup` in `lib/parts/src/cfgflash.rs` is the Rust
// model of this module, and the model below is the same one.
//
// Synthesised, the primitive drives the pin and `cclk` is held low,
// since the pin is not a net user logic can see. `eos` comes out of the
// primitive on no clock of ours, so it is registered twice.
//
// Simulated, there is no primitive, and the model does what UG470 says
// the primitive does: `eos` after EOS_AFTER cycles, and the first three
// rises of `usrcclko` after it spent moving the pin over to the user,
// never reaching `cclk`. Vivado defines SYNTHESIS when it synthesises
// and not when it simulates, which is what tells the two apart.
`timescale 1ns/1ps
module startup #(
    parameter EOS_AFTER = 8
) (
    input  wire clk,
    input  wire usrcclko,
    output wire eos,
    output wire cclk
);

`ifdef SYNTHESIS
  wire eos_raw;
  (* ASYNC_REG = "TRUE" *) reg [1:0] eos_sync = 2'b00;
  STARTUPE2 #(
      .PROG_USR("FALSE"),
      .SIM_CCLK_FREQ(0.0)
  ) startupe2 (
      .CFGCLK(),
      .CFGMCLK(),
      .EOS(eos_raw),
      .PREQ(),
      .CLK(1'b0),
      .GSR(1'b0),
      .GTS(1'b0),
      .KEYCLEARB(1'b1),
      .PACK(1'b0),
      .USRCCLKO(usrcclko),
      .USRCCLKTS(1'b0),
      .USRDONEO(1'b1),
      .USRDONETS(1'b1)
  );
  always @(posedge clk) eos_sync <= {eos_sync[0], eos_raw};
  assign eos  = eos_sync[1];
  assign cclk = 1'b0;
`else
  reg [3:0] count = 4'd0;
  reg       eos_r = 1'b0;
  reg       prev = 1'b0;
  reg [1:0] seen = 2'd0;
  reg       cclk_r = 1'b0;
  assign eos  = eos_r;
  assign cclk = cclk_r;
  always @(posedge clk) begin
    if (count != EOS_AFTER) count <= count + 4'd1;
    else eos_r <= 1'b1;
    prev <= usrcclko;
    if (eos_r && usrcclko && !prev && seen != 2'd3) seen <= seen + 2'd1;
    cclk_r <= usrcclko & eos_r & (seen == 2'd3);
  end
`endif
endmodule
