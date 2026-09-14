// SPDX-License-Identifier: Apache-2.0
// A reservation station of two inputs, written by hand in Verilog:
// the station ex_station runs in TxHDL, a byte and a half word by a
// two-bit tag, four lines, so that the two run side by side on the
// same offers and the build asserts that they agree. Each input's
// data is its tag above its value; the output is the tag above the
// two values. The widths are written out, as the lowered netlist has
// them; a parameter would serve the widths, not the count of inputs.
`timescale 1ns/1ps
// begin{ports}
module station_v(
  input clk,
  input [9:0] in0_data, input in0_valid, output in0_ready,
  input [17:0] in1_data, input in1_valid, output in1_ready,
  output [25:0] out_data, output out_valid, input out_ready
);
// end{ports}
// begin{state}
  // A cell per line for each input, and which cells hold.
  reg [7:0] mem0 [0:3];
  reg [15:0] mem1 [0:3];
  reg [3:0] occ0 = 0;
  reg [3:0] occ1 = 0;
  integer i;
  initial for (i = 0; i < 4; i = i + 1) begin
    mem0[i] = 0;
    mem1[i] = 0;
  end
// end{state}

// begin{reads}
  // What each input offers, and whether its cell is free.
  wire [1:0] tag0 = in0_data[9:8];
  wire [7:0] value0 = in0_data[7:0];
  wire [1:0] tag1 = in1_data[17:16];
  wire [15:0] value1 = in1_data[15:0];
  wire cell_free0 = ~occ0[tag0];
  wire cell_free1 = ~occ1[tag1];
// end{reads}

// begin{completes}
  // Taking an input completes its line when the other input's cell
  // of that line holds, or is offered now with a free cell.
  wire completes0 = in0_valid & cell_free0
    & (occ1[tag0] | (in1_valid & cell_free1 & (tag1 == tag0)));
  wire completes1 = in1_valid & cell_free1
    & (occ0[tag1] | (in0_valid & cell_free0 & (tag0 == tag1)));
// end{completes}

// begin{takes}
  // The line sent: the completing input's of lowest index, when the
  // output has room. An input is taken when its cell is free and it
  // completes nothing, or completes the line being sent.
  wire [1:0] line_tag = completes0 ? tag0 : tag1;
  wire send_line = (completes0 | completes1) & out_ready;
  wire take0 = in0_valid & cell_free0
    & (~completes0 | (send_line & (tag0 == line_tag)));
  wire take1 = in1_valid & cell_free1
    & (~completes1 | (send_line & (tag1 == line_tag)));
  assign in0_ready = take0;
  assign in1_ready = take1;
// end{takes}

// begin{drives}
  // A taken input fills its cell; the line sent clears its bit.
  wire [3:0] cell_bit0 = 4'b0001 << tag0;
  wire [3:0] cell_bit1 = 4'b0001 << tag1;
  wire [3:0] line_clear = send_line ? (4'b0001 << line_tag) : 4'b0000;
  always @(posedge clk) begin
    if (take0) mem0[tag0] <= value0;
    if (take1) mem1[tag1] <= value1;
    occ0 <= (occ0 | (take0 ? cell_bit0 : 4'b0000)) & ~line_clear;
    occ1 <= (occ1 | (take1 ? cell_bit1 : 4'b0000)) & ~line_clear;
  end
// end{drives}

// begin{outputs}
  // The line's values: from the cell if held, else from the input.
  wire [7:0] out0 = occ0[line_tag] ? mem0[line_tag] : value0;
  wire [15:0] out1 = occ1[line_tag] ? mem1[line_tag] : value1;
  assign out_data = {line_tag, out0, out1};
  assign out_valid = send_line;
// end{outputs}
endmodule
