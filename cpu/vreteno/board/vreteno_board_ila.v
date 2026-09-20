// SPDX-License-Identifier: Apache-2.0
// The board with a logic analyser in it (issue 239): this file is
// `vreteno_board.v` with one addition, an integrated logic analyser
// on the nets a reader of the board asks about first, sampled on the
// core's clock and read back over the JTAG cable with
// `//cpu/vreteno:vreteno_board_ila_read`. It is the same design
// otherwise, and it tracks the other file line for line; the
// difference is the block at the end. It is not the shipping
// bitstream: the analyser costs block RAM and a run of its own.
//
// What is probed, and why, in the order of the probes:
//
//   0  rst         the core's reset: the button, the lock, the
//                  calibration and the serial break, all in one
//   1  halt        the core has stopped
//   2  calib       the memory controller finished calibrating
//   3  uart_tx     what leaves on the serial line
//   4  rx_sync[1]  what arrives on it, synchronised
//   5  brk         the serial break that resets the core
//   6  pwm_pins    the modulator's four channels
//   7  vaw         the third slot's write address channel: a program
//                  that touches 0x3200 on this board waits here
//   8  var         its read address channel
//   9  vw          its write data channel
//  10  net_tx      the remote peripheral's frames, byte by byte
//  11  said        the serial line was driven at least once
//  12  mem_rst_n   the memory controller's reset
//
// The core's own retire stream, `instr` and `wb`, ends inside the
// lowered module and is not a port of it, so it is not here; that is
// the retire buffer of issue 240, a part on the bus rather than a
// probe. The memory's command pins are on the 400 MHz clock and would
// be sampled across a clock boundary, so they are left out too.
//
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
module vreteno_board_ila (
  input sys_clk_p,
  input sys_clk_n,
  input reset_n,
  // A user key on the carrier, KEY1, low while pressed, as a second
  // reset: the same as the RESET key, where a hand already is.
  input key1,
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
  // The core's reset waits on the memory's calibration as well, so the
  // core starts only when the memory it may reach is ready.
  //
  // The button resets the core and not the memory. The memory's reset
  // follows the PLL's lock alone, which is power-on. A reset that
  // reached the controller while a transaction was in flight stranded
  // the bus: the bridge in front of the controller waited on an
  // acknowledgement the reset controller never gave, nothing resets
  // that bridge, and every later access to the memory queued behind
  // it, so the loader took three words after a press and then nothing
  // (issue #333). With the memory running through the press, what was
  // in flight completes while the core is held, the core drains the
  // answers as it always does, and the bus is clean when it restarts.
  // The memory's contents survive a press, which is what a warm reset
  // means.
  //
  // The serial line held low resets the core as well: low for far
  // longer than any byte, 20 ms against a byte's 87 us, which no
  // program on the core produces and the host makes by sending one
  // zero byte at 300 baud, 30 ms of low. Not a break: a break through
  // this board's CP2102N, as Linux drives it, never reached the pin. It
  // gives the host a reset that no software on the core can miss,
  // because it is not software on the core, and it is what
  // `load --reset` uses (issue #332). The line is brought into the
  // design's clock through two flip-flops first. The reset is a pulse
  // when the count reaches the threshold rather than a level while the
  // line is low, so a line left floating low resets the core once and
  // not forever. The pulse is a millisecond, not a few cycles: a fetch
  // from the memory caught in flight answers long after a short pulse
  // has ended, and the core, back in the loader by then, would take
  // that answer as the reply to its first read of the serial port. A
  // finger on the button holds the reset for a tenth of a second and
  // never sees this; the break's reset has to be held on purpose.
  (* ASYNC_REG = "TRUE" *) reg [1:0] rx_sync = 2'b11;
  // The count is 22 bits: 2 100 000 does not fit in 21, and a literal
  // wider than its register is silently cut down, which made the
  // upper bound 2 848 and the pulse never come.
  reg [21:0] low_for = 0;
  reg brk = 0;
  always @(posedge clk) begin
    rx_sync <= {rx_sync[0], uart_rx};
    if (rx_sync[1]) low_for <= 0;
    else if (low_for != 22'h3fffff) low_for <= low_for + 1;
    brk <= (low_for >= 22'd2_000_000) && (low_for < 22'd2_100_000);
  end
  wire calib;
  reg [1:0] mem_sync = 2'b00;
  reg [1:0] core_sync = 2'b11;
  always @(posedge clk) begin
    mem_sync <= {mem_sync[0], locked};
    core_sync <= {core_sync[0], ~(reset_n & key1 & locked & calib) | brk};
  end
  wire mem_rst_n = mem_sync[1];
  wire rst = core_sync[1];

  // The design.
  wire halt;
  // The pulse width modulator's four channels. The first drives the
  // first LED, so a program can fade it; the other three go nowhere on
  // this board and are left for a design that wants them.
  wire [3:0] pwm_pins;
  // The third slot's channels and the remote peripheral's frames,
  // as nets so that the analyser can see them; the slot is still
  // unanswered and the frames still go nowhere, as on the board.
  wire [34:0] vaw_data, var_data;
  wire [35:0] vw_data;
  wire vaw_valid, var_valid, vw_valid;
  wire [8:0] net_tx_data;
  wire net_tx_valid;
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
    .vaw_data(vaw_data), .vaw_valid(vaw_valid), .vaw_ready(1'b1),
    .var_data(var_data), .var_valid(var_valid), .var_ready(1'b1),
    .vw_data(vw_data), .vw_valid(vw_valid), .vw_ready(1'b1),
    .vb_data(2'd0), .vb_valid(1'b0), .vb_ready(),
    .vr_data(34'd0), .vr_valid(1'b0), .vr_ready(),
    // The remote peripheral at `0x3300` sends its transactions out as
    // Ethernet frames, and this board has no Ethernet port: the
    // frames go nowhere and none come back, so a program that touches
    // `0x3300` waits the peripheral's patience out and is told the
    // device failed. //flagship is the board with the port.
    .net_tx_data(net_tx_data), .net_tx_valid(net_tx_valid), .net_tx_ready(1'b1),
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

  // The analyser: `//cpu/vreteno:board_ila`, generated by Vivado with
  // these thirteen probes at these widths and 4096 samples deep. The
  // list at the top of this file is the list here.
  board_ila probes (
    .clk(clk),
    .probe0(rst),
    .probe1(halt),
    .probe2(calib),
    .probe3(uart_tx),
    .probe4(rx_sync[1]),
    .probe5(brk),
    .probe6(pwm_pins),
    .probe7({vaw_valid, vaw_data}),
    .probe8({var_valid, var_data}),
    .probe9({vw_valid, vw_data}),
    .probe10({net_tx_valid, net_tx_data}),
    .probe11(said),
    .probe12(mem_rst_n)
  );
endmodule
