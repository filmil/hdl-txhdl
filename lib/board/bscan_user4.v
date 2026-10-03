// SPDX-License-Identifier: Apache-2.0
// The device's fourth user scan chain, USER4, where OpenOCD's BSCAN
// tunnel reaches the RISC-V debug transport (issue 154). Under
// synthesis it is BSCANE2 with JTAG_CHAIN 4, its clock on a global
// buffer; in simulation no cable is there, so every pin stays low.
module bscan_user4 (
  output tck,
  output sel,
  output shift,
  output capture,
  output update,
  output tdi,
  output reset,
  input tdo
);
`ifdef SYNTHESIS
  wire tck_raw;
  BSCANE2 #(.JTAG_CHAIN(4)) bscan (
    .CAPTURE(capture), .DRCK(), .RESET(reset), .RUNTEST(), .SEL(sel),
    .SHIFT(shift), .TCK(tck_raw), .TDI(tdi), .TMS(), .UPDATE(update),
    .TDO(tdo)
  );
  BUFG tck_buf (.I(tck_raw), .O(tck));
`else
  assign {tck, sel, shift, capture, update, tdi, reset} = 7'd0;
`endif
endmodule
