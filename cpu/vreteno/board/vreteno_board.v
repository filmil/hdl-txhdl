// SPDX-License-Identifier: Apache-2.0
//
// The Vreteno board on the Alinx AX7A200: the pin and clock wrapper
// around the lowered design, and nothing else.
//
// Everything that computes is `board`, the netlist `#[lower]` writes for
// `vreteno32::board::Board`: the core, the router, the data memory, the
// timer, the serial port and the DDR3 memory with UberDDR3's controller
// in it. What a board needs and a lowered unit cannot say is here: the
// 200 MHz differential clock turned into the four clocks the design and
// the controller run on, the resets, and the LEDs.
//
// The clocks come from one PLL, as the a200t examples make them: the
// 200 MHz input times six is 1200 MHz, which gives 100 MHz for the
// design and the controller, 400 MHz for the memory, the same 400 MHz a
// quarter cycle late, and 200 MHz for the controller's delay control.
//
// The memory's reset follows the button and the PLL's lock. The core's
// reset follows those and the controller's calibration as well, so the
// core starts only when the memory it may reach is ready. Both are
// brought into the design's clock by two flip-flops.
//
// The LEDs are lit when driven low: the core halted, the serial line
// driven at least once, the memory calibrated, and a heartbeat.
//
// The second LED carries what the heartbeat used to, and the heartbeat
// moved to the fourth. The second now latches the first start bit on the
// serial line, which is the open question of issue #195: the board halts
// with every LED on and nothing arrives on the host's serial port, and
// nothing so far says whether a byte ever left the core.
//
// The fourth LED showed the PLL's lock. A blinking heartbeat says that
// as well, since the counter behind it runs on the PLL's output, and a
// heartbeat in a new place is also how somebody at the board can tell
// that a new bitstream is in the part.
`timescale 1ps / 1ps
module vreteno_board (
  input sys_clk_p,
  input sys_clk_n,
  input reset_n,
  output led1,
  output led2,
  output led3,
  output led4,
  output uart_tx,
  input uart_rx,
  output [14:0] ddr3_a,
  output [2:0] ddr3_ba,
  output ddr3_ras,
  output ddr3_cas,
  output ddr3_we,
  output ddr3_s0,
  output ddr3_cke,
  output ddr3_odt,
  output ddr3_reset,
  output ddr3_clk_p,
  output ddr3_clk_n,
  output [3:0] ddr3_dm,
  inout [31:0] ddr3_dq_p,
  inout [3:0] ddr3_dqs_p,
  inout [3:0] ddr3_dqs_n
);
  // The clocks.
  wire clk200_in, fb, fb_buf, locked;
  wire pll100, pll400, pll200, pll400_90;
  wire clk, clk400, clk200, clk400_90;
  IBUFDS clkin (.I(sys_clk_p), .IB(sys_clk_n), .O(clk200_in));
  PLLE2_BASE #(
    .BANDWIDTH("OPTIMIZED"),
    .CLKIN1_PERIOD(5.0),
    .CLKFBOUT_MULT(6),
    .CLKOUT0_DIVIDE(12),
    .CLKOUT1_DIVIDE(3),
    .CLKOUT2_DIVIDE(6),
    .CLKOUT3_DIVIDE(3),
    .CLKOUT3_PHASE(90.0),
    .DIVCLK_DIVIDE(1),
    .STARTUP_WAIT("FALSE")
  ) pll (
    .CLKIN1(clk200_in),
    .CLKFBIN(fb_buf),
    .CLKFBOUT(fb),
    .CLKOUT0(pll100),
    .CLKOUT1(pll400),
    .CLKOUT2(pll200),
    .CLKOUT3(pll400_90),
    .CLKOUT4(),
    .CLKOUT5(),
    .LOCKED(locked),
    .PWRDWN(1'b0),
    .RST(1'b0)
  );
  BUFG fbbuf (.I(fb), .O(fb_buf));
  BUFG buf100 (.I(pll100), .O(clk));
  BUFG buf400 (.I(pll400), .O(clk400));
  BUFG buf200 (.I(pll200), .O(clk200));
  BUFG buf400_90 (.I(pll400_90), .O(clk400_90));

  // The resets, active high for the design and active low for the
  // controller, each through two flip-flops into the design's clock.
  wire calib;
  reg [1:0] mem_sync = 2'b00;
  reg [1:0] core_sync = 2'b11;
  always @(posedge clk) begin
    mem_sync <= {mem_sync[0], reset_n & locked};
    core_sync <= {core_sync[0], ~(reset_n & locked & calib)};
  end
  wire mem_rst_n = mem_sync[1];
  wire rst = core_sync[1];

  // The design.
  wire halt;
  // The pulse width modulator's four channels. The first drives the
  // first LED, so a program can fade it; the other three go nowhere on
  // this board and are left for a design that wants them.
  wire [3:0] pwm_pins;
  board lowered (
    .clk(clk),
    .rst(rst),
    .irq(1'b0),
    .rx(uart_rx),
    .ddr3_clk(clk400),
    .ref_clk(clk200),
    .ddr3_clk_90(clk400_90),
    .ddr3_rst_n(mem_rst_n),
    .halt(halt),
    .tx(uart_tx),
    .pwm_pins(pwm_pins),
    .calib(calib),
    .ck_p(ddr3_clk_p),
    .ck_n(ddr3_clk_n),
    .mem_rst_n(ddr3_reset),
    .cke(ddr3_cke),
    .cs_n(ddr3_s0),
    .ras_n(ddr3_ras),
    .cas_n(ddr3_cas),
    .we_n(ddr3_we),
    .row(ddr3_a),
    .bank(ddr3_ba),
    .dm(ddr3_dm),
    .odt(ddr3_odt),
    .dq(ddr3_dq_p),
    .dqs(ddr3_dqs_p),
    .dqs_n(ddr3_dqs_n),
    // The third slot of the page at `0x3000`, at `0x3200`, is for a
    // peripheral on a clock of its own, and this board has none: its
    // requests go nowhere and no answer comes back, so a program that
    // touches `0x3200` waits forever, and nothing here touches it.
    // //flagship is the board that fills the slot, with the video
    // peripheral on the pixel clock.
    .vaw_data(), .vaw_valid(), .vaw_ready(1'b1),
    .var_data(), .var_valid(), .var_ready(1'b1),
    .vw_data(), .vw_valid(), .vw_ready(1'b1),
    .vb_data(2'd0), .vb_valid(1'b0), .vb_ready(),
    .vr_data(34'd0), .vr_valid(1'b0), .vr_ready(),
    // The remote peripheral at `0x3300` sends its transactions out as
    // Ethernet frames, and this board has no Ethernet port: the
    // frames go nowhere and none come back, so a program that touches
    // `0x3300` waits the peripheral's patience out and is told the
    // device failed. //flagship is the board with the port.
    .net_tx_data(), .net_tx_valid(), .net_tx_ready(1'b1),
    .net_rx_data(9'd0), .net_rx_valid(1'b0), .net_rx_ready()
  );

  // The serial line idles high, so any byte begins by pulling it low.
  // This latch holds from the first start bit until the next reset,
  // which is what separates a design that never drove the line from a
  // board path that does not carry it.
  reg said = 0;
  always @(posedge clk) begin
    if (rst) said <= 1'b0;
    else if (!uart_tx) said <= 1'b1;
  end

  // The heartbeat: bit 25 of a counter at 100 MHz toggles three times a
  // second.
  reg [25:0] beat = 0;
  always @(posedge clk) beat <= beat + 1;

  // The first LED is the modulator's first channel rather than the
  // core's halt: a program sets its duty and the light follows. The
  // halt still shows, on the same LED, by holding it on once the core
  // stops, since a stopped core leaves the channel wherever it was.
  assign led1 = ~(pwm_pins[0] | halt);
  assign led2 = ~said;
  assign led3 = ~calib;
  assign led4 = ~beat[25];
endmodule
