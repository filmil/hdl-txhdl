// SPDX-License-Identifier: Apache-2.0
// `axi_count`, `count` with its port marked `#[axi]`, joined to `stage`
// wire to wire as `direct_tb` joins `count` (#1871). Its valid does not
// wait on its ready, so only `stage`'s ready waits on the other side,
// which AXI allows: there is no loop, and the words pass in order, a
// word a cycle once reset is let go. No -Wno-fatal: a loop would fail
// the build.
module direct_axi_tb;
  reg clk = 0;
  reg rst = 1;
  wire [7:0] d, od;
  wire v, r, ov;
  axi_count u_count(.clk(clk), .rst(rst), .out_data(d), .out_valid(v),
                    .out_ready(r));
  stage u_stage(.clk(clk), .rst(rst), .inp_data(d), .inp_valid(v),
                .inp_ready(r), .out_data(od), .out_valid(ov),
                .out_ready(1'b1));
  integer passed = 0;
  integer cycle;
  initial begin
    // The clock rises at 5, 16, ...; reset is let go on the falling
    // edge, and the outputs are looked at a nanosecond later, once the
    // wires have settled. `stage` passes a word through in the cycle it
    // is offered, so what is seen here is the word the next rising edge
    // takes.
    for (cycle = 0; cycle < 100; cycle = cycle + 1) begin
      #5 clk = 1;
      #5 clk = 0;
      if (cycle == 2) rst = 0;
      #1;
      // AXI keeps a source's valid low through a reset.
      if (rst && v) begin
        $display("direct_axi: valid high in reset at cycle %0d", cycle);
        $fatal(1);
      end
      if (ov) begin
        if (od != passed[7:0]) begin
          $display("direct_axi: word %0d was %0d", passed, od);
          $fatal(1);
        end
        passed = passed + 1;
      end
    end
    if (passed < 95) begin
      $display("direct_axi: only %0d words passed", passed);
      $fatal(1);
    end
    $display("direct_axi: %0d words passed, in order", passed);
    $finish;
  end
endmodule
