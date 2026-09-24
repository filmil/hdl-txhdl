// SPDX-License-Identifier: Apache-2.0
//
// The physical half of the entropy source: N ring oscillators, each
// a loop of an odd number of inverters through one gate that holds
// it still while `en` is low, sampled by a flop on `clk`. Each ring
// runs at a frequency set by its own gates and wiring, drifting with
// temperature and supply, so where the sample lands in its period is
// the jitter that is the randomness. Sampling one ring gives a biased
// bit; the peripheral folds the N samples together and debiases what
// is left.
//
// Vivado would fold a loop of inverters to nothing, or merge the N
// identical rings into one, so every gate is kept and every loop is
// allowed. The constraints in `ring_osc.xdc` tell the timer to leave
// the loops alone, since a loop has no period a timer can meet.
//
// Under a simulator a combinational loop of zero-delay gates is an
// infinite event loop, so when `SYNTHESIS` is not defined the rings
// are replaced by a shift register with feedback of its own seed per
// ring: a stream that looks like a ring's samples and is nothing of
// the kind. Vivado defines `SYNTHESIS` when it synthesises and not
// when it simulates, which is what tells the two apart.
module ring_osc #(
    parameter N = 8,
    parameter L = 7
) (
    input  wire         clk,
    input  wire         en,
    output reg  [N-1:0] raw
);

`ifdef SYNTHESIS
  genvar i, j;
  generate
    for (i = 0; i < N; i = i + 1) begin : ring
      // The loop: an AND of the enable, then L inverters, back to the
      // AND. L odd makes the loop oscillate; the attributes keep every
      // gate as its own LUT and stop Vivado objecting to the loop.
      (* DONT_TOUCH = "TRUE", ALLOW_COMBINATORIAL_LOOPS = "TRUE" *)
      wire [L:0] chain;
      (* DONT_TOUCH = "TRUE" *)
      LUT2 #(.INIT(4'h8)) gate (
          .O(chain[0]),
          .I0(en),
          .I1(chain[L])
      );
      for (j = 0; j < L; j = j + 1) begin : inv
        (* DONT_TOUCH = "TRUE" *)
        LUT1 #(.INIT(2'h1)) not_j (
            .O(chain[j+1]),
            .I0(chain[j])
        );
      end
      // The sample. The flop may go metastable, which is the point of
      // it; the peripheral registers the bit again before it reads it.
      (* ASYNC_REG = "TRUE" *)
      reg sample;
      always @(posedge clk) begin
        sample <= chain[L];
        raw[i] <= sample;
      end
    end
  endgenerate
`else
  // The simulation model: a 32-bit maximal linear feedback shift
  // register per ring, seeded apart, and a bit of each per cycle.
  genvar i;
  generate
    for (i = 0; i < N; i = i + 1) begin : ring
      reg [31:0] lfsr;
      // The seeds, one a ring and apart from each other, as `SEEDS` in
      // `lib/parts/src/trng.rs` has them for the Rust model.
      initial case (i % 8)
        0: lfsr = 32'h9e37_79b9;
        1: lfsr = 32'h7f4a_7c15;
        2: lfsr = 32'h2545_f491;
        3: lfsr = 32'h6c8e_9cf5;
        4: lfsr = 32'h1b87_3593;
        5: lfsr = 32'hcc9e_2d51;
        6: lfsr = 32'h85eb_ca6b;
        default: lfsr = 32'hc2b2_ae35;
      endcase
      always @(posedge clk) begin
        if (en) begin
          lfsr <= {lfsr[30:0], lfsr[31] ^ lfsr[21] ^ lfsr[1] ^ lfsr[0]};
          raw[i] <= lfsr[31];
        end else begin
          raw[i] <= 1'b0;
        end
      end
    end
  endgenerate
`endif

endmodule
