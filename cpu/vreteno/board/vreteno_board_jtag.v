// SPDX-License-Identifier: Apache-2.0
// The board with a JTAG master on its bus (issue 241): this file is
// `vreteno_board.v` with one addition, Vivado's JTAG-to-AXI master,
// `//cpu/vreteno:jtag_axi`, on the second host port of the lowered
// board, reached over the cable that programs the part. From the
// hardware manager, `create_hw_axi_txn` reads and writes every address
// the board's map names, whatever the core is doing: the data memory,
// the timer, the serial port, the interrupt controller, the DDR3, the
// boot memory. It tracks the other file line for line; the difference
// is the block at the end and the pins it drives.
//
//
// The Vreteno board on the Alinx AX7A200: the pin and clock wrapper
// around the lowered design, and nothing else.
//
// Everything that computes is `board`, the netlist `#[lower]` writes for
// `vreteno32::board::Board`: the core, the router, the data memory, the
// timer, the serial port and the DDR3 memory with AMD's MIG controller
// in it. What a board needs and a lowered unit cannot say is here: the
// 200 MHz differential clock handed to the controller, which makes the
// design's clock from it, the resets, and the LEDs.
//
// The controller makes the clocks: it takes the board's 200 MHz, which
// is also its delay reference, runs the memory at 400 MHz, and hands
// back the 100 MHz the design runs on, with a reset that holds until
// that clock is good. The board has no clock generator of its own.
//
// The controller's reset is a pulse at power-on and nothing else. The
// core's reset follows the button, the controller's clock being good
// and its calibration, so the core starts only when the memory it may
// reach is ready, brought into the design's clock by two flip-flops.
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
// The fourth LED showed the clock generator's lock. A blinking heartbeat
// says that as well, since the counter behind it runs on the clock the
// controller makes, and a
// heartbeat in a new place is also how somebody at the board can tell
// that a new bitstream is in the part.
`timescale 1ps / 1ps
module vreteno_board_jtag (
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
  // The clocks. The memory controller makes them: it takes the board's
  // 200 MHz and hands back `clk`, the 100 MHz the design runs on, with
  // `ui_rst` high until that clock is good. The controller's own reset
  // is a pulse at power-on, counted on the board's clock, and nothing
  // else.
  wire clk200_in, clk, ui_rst;
  IBUFDS clkin (.I(sys_clk_p), .IB(sys_clk_n), .O(clk200_in));
  reg [7:0] por = 8'd0;
  always @(posedge clk200_in) if (por != 8'hff) por <= por + 1;
  wire sys_rst = (por != 8'hff);

  // The reset, active high for the design, through two flip-flops into
  // the design's clock.
  // The core's reset waits on the memory's calibration as well, so the
  // core starts only when the memory it may reach is ready.
  //
  // The button resets the core and not the memory. The controller's
  // reset is the power-on pulse alone. A reset that
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
  reg [1:0] core_sync = 2'b11;
  always @(posedge clk) begin
    core_sync <= {core_sync[0], ~(reset_n & key1 & ~ui_rst & calib) | brk};
  end
  wire rst = core_sync[1];

  // The design.
  wire halt;
  // The pulse width modulator's four channels. The first drives the
  // first LED, so a program can fade it; the other three go nowhere on
  // this board and are left for a design that wants them.
  wire [3:0] pwm_pins;
  // The JTAG master's pins, between it and the lowered board. Its
  // reset is active low and the board's is active high.
  wire j_awid, j_arid, j_awvalid, j_awready, j_wvalid, j_wready;
  wire j_bvalid, j_bready, j_arvalid, j_arready, j_rvalid, j_rready;
  wire j_awlock, j_arlock, j_wlast, j_rlast;
  wire [31:0] j_awaddr, j_araddr, j_wdata, j_rdata;
  wire [7:0] j_awlen, j_arlen;
  wire [2:0] j_awsize, j_arsize, j_awprot, j_arprot;
  wire [1:0] j_awburst, j_arburst, j_bresp, j_rresp, j_bid, j_rid;
  wire [3:0] j_awcache, j_arcache, j_wstrb;
  board lowered (
    .clk(clk),
    .rst(rst),
    .irq(1'b0),
    .rx(uart_rx),
    .sys_clk(clk200_in),
    .sys_rst(sys_rst),
    .halt(halt),
    .tx(uart_tx),
    .pwm_pins(pwm_pins),
    .calib(calib),
    .ui_clk(clk),
    .ui_rst(ui_rst),
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
    .net_rx_data(9'd0), .net_rx_valid(1'b0), .net_rx_ready(),
    // The JTAG master's pins (issue 241), driven by the master below.
    // The master's identifier is one bit; the board's host ports carry
    // two, so it is widened with a zero and narrowed back.
    .jtag_awid({1'b0, j_awid}), .jtag_awaddr(j_awaddr),
    .jtag_awlen(j_awlen), .jtag_awsize(j_awsize),
    .jtag_awburst(j_awburst), .jtag_awlock(j_awlock),
    .jtag_awcache(j_awcache), .jtag_awprot(j_awprot),
    .jtag_awvalid(j_awvalid), .jtag_awready(j_awready),
    .jtag_wdata(j_wdata), .jtag_wstrb(j_wstrb), .jtag_wlast(j_wlast),
    .jtag_wvalid(j_wvalid), .jtag_wready(j_wready),
    .jtag_bid(j_bid), .jtag_bresp(j_bresp), .jtag_bvalid(j_bvalid),
    .jtag_bready(j_bready),
    .jtag_arid({1'b0, j_arid}), .jtag_araddr(j_araddr),
    .jtag_arlen(j_arlen), .jtag_arsize(j_arsize),
    .jtag_arburst(j_arburst), .jtag_arlock(j_arlock),
    .jtag_arcache(j_arcache), .jtag_arprot(j_arprot),
    .jtag_arvalid(j_arvalid), .jtag_arready(j_arready),
    .jtag_rid(j_rid), .jtag_rdata(j_rdata), .jtag_rresp(j_rresp),
    .jtag_rlast(j_rlast), .jtag_rvalid(j_rvalid), .jtag_rready(j_rready)
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

  // The master: `//cpu/vreteno:jtag_axi`, generated by Vivado, AXI4
  // with no bursts, since every peripheral on this bus serves one beat
  // at a time. Its QoS pins go nowhere, as the board's host pins have
  // none.
  jtag_axi master (
    .aclk(clk), .aresetn(~rst),
    .m_axi_awid(j_awid), .m_axi_awaddr(j_awaddr), .m_axi_awlen(j_awlen),
    .m_axi_awsize(j_awsize), .m_axi_awburst(j_awburst),
    .m_axi_awlock(j_awlock), .m_axi_awcache(j_awcache),
    .m_axi_awprot(j_awprot), .m_axi_awqos(),
    .m_axi_awvalid(j_awvalid), .m_axi_awready(j_awready),
    .m_axi_wdata(j_wdata), .m_axi_wstrb(j_wstrb), .m_axi_wlast(j_wlast),
    .m_axi_wvalid(j_wvalid), .m_axi_wready(j_wready),
    .m_axi_bid(j_bid[0]), .m_axi_bresp(j_bresp), .m_axi_bvalid(j_bvalid),
    .m_axi_bready(j_bready),
    .m_axi_arid(j_arid), .m_axi_araddr(j_araddr), .m_axi_arlen(j_arlen),
    .m_axi_arsize(j_arsize), .m_axi_arburst(j_arburst),
    .m_axi_arlock(j_arlock), .m_axi_arcache(j_arcache),
    .m_axi_arprot(j_arprot), .m_axi_arqos(),
    .m_axi_arvalid(j_arvalid), .m_axi_arready(j_arready),
    .m_axi_rid(j_rid[0]), .m_axi_rdata(j_rdata), .m_axi_rresp(j_rresp),
    .m_axi_rlast(j_rlast), .m_axi_rvalid(j_rvalid), .m_axi_rready(j_rready)
  );
endmodule
