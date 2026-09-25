// SPDX-License-Identifier: Apache-2.0
//
// The flagship on the Alinx AX7A200B: every part of the system that is
// proven on this board, in one bitstream, written to the QSPI flash so
// that the board comes up as itself after a power cycle. See #224.
//
// What is in it, and what each part answers for:
//
//   * The Vreteno core with its router, data memory, timer, interrupt
//     controller, pulse width modulator, serial port and the board's
//     DDR3 memory: `board`, the netlist `#[lower]` writes for
//     `vreteno32::board::Board`, with the loader in the core's boot
//     memory. The core does not come up running a program built into
//     the part. It comes up waiting on the serial port, so the software
//     on the flagship changes in a second and the bitstream underneath
//     it does not move. That is the flagship's point: the hardware is
//     fixed and known, and the software is whatever was sent last.
//   * The Ethernet port: the two lowered halves of the MAC with the
//     PHY's RGMII, and behind them the remote peripheral at `0x3300`,
//     whose behaviour is a program on another machine. A transaction
//     the core makes there leaves as one frame and the answer arrives
//     as one, so a device can be written as software on a laptop and
//     the design does not change when it becomes hardware. See #297.
//     The port echoed every good frame until this; a frame cannot be
//     both echoed and answered, and what proves the port now is a
//     program answering at the other end of the cable.
//   * The HDMI output: the video peripheral with a picture already in
//     its framebuffer, and the master that configures the encoder.
//     The core reaches the peripheral, so a program can paint the
//     screen; the picture in the netlist is what shows until one does.
//
// The clocks. One 200 MHz differential input, one IBUFDS, and three
// generators hanging off it, because the three subsystems want
// unrelated frequencies and each is already proven with the generator
// it has:
//
//   * the memory controller's own: it takes the 200 MHz, which is also
//     its delay reference, runs the memory at 400 MHz and hands back
//     100 MHz for the design and the serial port. This is
//     `vreteno_board.v`'s arrangement, unchanged.
//   * an MMCM, times five to 1000 MHz over eight: 125 MHz for the
//     Ethernet transmit half. This is `eth_echo.v`'s, unchanged. A
//     second MMCM inside `eth_rgmii` shifts the PHY's receive clock.
//   * an MMCM, times 63 over ten and then over 50: 25.2 MHz for the
//     pixel clock. This is `hdmi_demo.v`'s, unchanged.
//
// Two things cross between the domains, and both are channels rather
// than signals. The core's third AXI-Lite slot, the page at `0x3200`,
// leaves `board` as five channel ports and reaches the video
// peripheral on the pixel clock through five `chan_cdc` FIFOs, one per
// AXI-Lite channel, each carrying its words in the direction that
// channel runs. Gray-coded pointers through two flip-flops on each
// side, which is the same crossing the Ethernet half already uses on
// this board. The five channels of AXI-Lite have no timing
// relationship to each other, so crossing each on its own is the whole
// of what the protocol asks.
//
// The other is the remote peripheral's frames, one byte and a last bit
// at a time: out on the core's clock to the transmit clock, and in on
// the PHY's receive clock to the core's. Two more of the same FIFO,
// and the one going in is 128 bytes deep because the port delivers a
// byte every 8 ns and the core reads one every 10.
//
// Nothing else crosses: the Ethernet and the memory share the board,
// the reset button and the LEDs with the rest, and nothing more.
//
// The LEDs are lit when driven low. Four lights and more than four
// things worth watching, so they are the four a person at the board
// needs to tell a healthy flagship from a sick one:
//
//   led1  the modulator's first channel, or the core halted. This is
//         the software's own light: the loader leaves it dark, and a
//         program that was sent down the wire does with it what it
//         likes.
//   led2  a frame seen on the Ethernet port, either direction, held
//         for about a fifth of a second.
//   led3  the DDR3 memory calibrated. The loader writes a program into
//         that memory, so a dark led3 explains everything else.
//   led4  the heartbeat, three times a second, which says the memory
//         controller's clock is good and this part holds a bitstream
//         that is running.
//
// The HDMI has no light of its own; the monitor is its light.
`timescale 1ps / 1ps
module flagship (
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
  // The serial port: the loader listens here.
  output uart_tx,
  input uart_rx,
  // The memory.
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
  inout [3:0] ddr3_dqs_n,
  // The Ethernet PHY.
  output eth_txck,
  output eth_txctl,
  output [3:0] eth_txd,
  input eth_rxck,
  input eth_rxctl,
  input [3:0] eth_rxd,
  output eth_reset_n,
  output eth_mdc,
  inout eth_mdio,
  // The HDMI encoder.
  output hdmi_nreset,
  // The same reset on the ball the other board revision wires it to;
  // see issue #197.
  output hdmi_nreset_alt,
  output hdmi_clk,
  output hdmi_hs,
  output hdmi_vs,
  output hdmi_de,
  output [23:0] hdmi_d,
  output hdmi_scl,
  inout hdmi_sda
);
  // The one input clock, for all three generators.
  wire clk200_in;
  IBUFDS clkin (.I(sys_clk_p), .IB(sys_clk_n), .O(clk200_in));

  // --------------------------------------------------------------
  // The core, the memory and the serial port.
  // --------------------------------------------------------------

  // The memory controller makes the design's clock: it takes the 200 MHz
  // and hands back `clk`, the 100 MHz the core, the memory and the
  // serial port run on, with `ui_rst` high until that clock is good.
  // The controller's own reset is a pulse at power-on, counted on the
  // board's clock, and nothing else.
  wire clk, ui_rst;
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

  wire halt;
  wire [3:0] pwm_pins;
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
    // The video slot at `0x3200`, on the core's clock, crossed below.
    .vaw_data(vaw_data), .vaw_valid(vaw_valid), .vaw_ready(vaw_ready),
    .var_data(var_data), .var_valid(var_valid), .var_ready(var_ready),
    .vw_data(vw_data), .vw_valid(vw_valid), .vw_ready(vw_ready),
    .vb_data(vb_data), .vb_valid(vb_valid), .vb_ready(vb_ready),
    .vr_data(vr_data), .vr_valid(vr_valid), .vr_ready(vr_ready),
    // The remote peripheral at `0x3300`, whose frames leave and
    // arrive on the Ethernet port, crossed below.
    .net_tx_data(net_tx_data), .net_tx_valid(net_tx_valid),
    .net_tx_ready(net_tx_ready),
    .net_rx_data(net_rx_data), .net_rx_valid(net_rx_valid),
    .net_rx_ready(net_rx_ready),
    // The JTAG master's pins (issue 241). This top has no master on
    // them: every valid low, every ready low, and the answers unread.
    // //cpu/vreteno:vreteno_board_jtag_pnr is the top that has one.
    .jtag_awid(2'd0), .jtag_awaddr(32'd0), .jtag_awlen(8'd0),
    .jtag_awsize(3'd0), .jtag_awburst(2'd0), .jtag_awlock(1'b0),
    .jtag_awcache(4'd0), .jtag_awprot(3'd0), .jtag_awvalid(1'b0),
    .jtag_wdata(32'd0), .jtag_wstrb(4'd0), .jtag_wlast(1'b0),
    .jtag_wvalid(1'b0), .jtag_bready(1'b0),
    .jtag_arid(2'd0), .jtag_araddr(32'd0), .jtag_arlen(8'd0),
    .jtag_arsize(3'd0), .jtag_arburst(2'd0), .jtag_arlock(1'b0),
    .jtag_arcache(4'd0), .jtag_arprot(3'd0), .jtag_arvalid(1'b0),
    .jtag_rready(1'b0),
    .jtag_awready(), .jtag_wready(), .jtag_bid(), .jtag_bresp(),
    .jtag_bvalid(), .jtag_arready(), .jtag_rid(), .jtag_rdata(),
    .jtag_rresp(), .jtag_rlast(), .jtag_rvalid()
  );

  // --------------------------------------------------------------
  // The Ethernet port.
  // --------------------------------------------------------------

  wire eth_fb, eth_fb_buf, eth_locked, mmcm125, clk125;
  MMCME2_BASE #(
    .CLKIN1_PERIOD(5.0),
    .DIVCLK_DIVIDE(1),
    .CLKFBOUT_MULT_F(5.0),
    .CLKOUT0_DIVIDE_F(8.0)
  ) eth_mmcm (
    .CLKIN1(clk200_in),
    .CLKFBIN(eth_fb_buf),
    .CLKFBOUT(eth_fb),
    .CLKOUT0(mmcm125),
    .CLKOUT1(),
    .CLKOUT0B(), .CLKOUT1B(), .CLKOUT2(), .CLKOUT2B(), .CLKOUT3(),
    .CLKOUT3B(), .CLKOUT4(), .CLKOUT5(), .CLKOUT6(), .CLKFBOUTB(),
    .LOCKED(eth_locked),
    .PWRDWN(1'b0),
    .RST(1'b0)
  );
  BUFG eth_fb_bufg (.I(eth_fb), .O(eth_fb_buf));
  BUFG clk125_bufg (.I(mmcm125), .O(clk125));

  wire rx_clk;
  wire [7:0] txd, rxd;
  wire tx_en, rx_dv, rx_er;
  // A reset for each half of the MAC, on the clock that half runs on.
  // What asks for it is what asks for the core's: the button, the key
  // and the break on the serial line, and besides those the Ethernet
  // clock manager's lock, since the halves are nothing until their
  // clocks are. Each goes through two flip-flops in its own domain, so
  // that its release lands on an edge of the clock it releases; the
  // core's `rst` is not wired across, since it is on the core's clock
  // (issue 509). The receive clock is the PHY's and does not run until
  // the PHY does, so its synchroniser holds the reset from its initial
  // value until the first edges arrive.
  wire eth_rst_ask = ~(reset_n & key1 & eth_locked) | brk;
  (* ASYNC_REG = "TRUE" *) reg [1:0] rx_rst_sync = 2'b11;
  (* ASYNC_REG = "TRUE" *) reg [1:0] tx_rst_sync = 2'b11;
  always @(posedge rx_clk) rx_rst_sync <= {rx_rst_sync[0], eth_rst_ask};
  always @(posedge clk125) tx_rst_sync <= {tx_rst_sync[0], eth_rst_ask};
  wire rx_rst = rx_rst_sync[1];
  wire tx_rst = tx_rst_sync[1];

  eth_rgmii rgmii (
    .clk125(clk125), .txd(txd), .tx_en(tx_en),
    .rx_clk(rx_clk), .rxd(rxd), .rx_dv(rx_dv), .rx_er(rx_er),
    .eth_txck(eth_txck), .eth_txctl(eth_txctl), .eth_txd(eth_txd),
    .eth_rxck(eth_rxck), .eth_rxctl(eth_rxctl), .eth_rxd(eth_rxd)
  );
  // The PHY comes out of reset once its clock is steady, and the
  // management interface stays idle, so the JL2121 runs on its
  // power-on configuration.
  assign eth_reset_n = eth_locked;
  assign eth_mdc = 1'b1;
  assign eth_mdio = 1'bz;

  wire [8:0] rx_data;
  wire rx_valid, rx_ready;
  eth_rx mac_rx (
    .clk(rx_clk),
    .rst(rx_rst),
    .rxd(rxd), .rx_dv(rx_dv), .rx_er(rx_er),
    .rx_data(rx_data), .rx_valid(rx_valid), .rx_ready(rx_ready)
  );

  // The received frames cross to the core's clock, where the remote
  // peripheral's link reads them. The FIFO is 128 bytes rather than
  // the echo's 16, because the port delivers a byte every 8 ns and the
  // core reads one every 10, so a whole frame has to fit while the
  // reader catches up. A frame that arrives while the last one is
  // still coming out of the MAC is lost, and a lost frame is a
  // transaction the peripheral's patience covers.
  wire [8:0] net_rx_data;
  wire net_rx_valid, net_rx_ready;
  chan_cdc #(.W(9), .AW(7)) rx_crossing (
    .wr_clk(rx_clk), .wr_data(rx_data), .wr_valid(rx_valid),
    .wr_ready(rx_ready),
    .rd_clk(clk), .rd_data(net_rx_data), .rd_valid(net_rx_valid),
    .rd_ready(net_rx_ready)
  );

  // And the frames the peripheral sends cross the other way, to the
  // transmit clock. The MAC stores a frame whole before it puts it on
  // the wire, so it takes a byte a cycle and this side never fills.
  wire [8:0] net_tx_data;
  wire net_tx_valid, net_tx_ready;
  wire [8:0] tx_data;
  wire tx_valid, tx_ready;
  chan_cdc #(.W(9), .AW(4)) tx_crossing (
    .wr_clk(clk), .wr_data(net_tx_data), .wr_valid(net_tx_valid),
    .wr_ready(net_tx_ready),
    .rd_clk(clk125), .rd_data(tx_data), .rd_valid(tx_valid),
    .rd_ready(tx_ready)
  );

  eth_tx mac_tx (
    .clk(clk125),
    .rst(tx_rst),
    .tx_data(tx_data), .tx_valid(tx_valid), .tx_ready(tx_ready),
    .txd(txd), .tx_en(tx_en)
  );

  // --------------------------------------------------------------
  // The HDMI output.
  // --------------------------------------------------------------

  wire vid_fb, vid_fb_buf, vid_locked, mmcm25, pixclk;
  MMCME2_BASE #(
    .CLKIN1_PERIOD(5.0),
    .DIVCLK_DIVIDE(10),
    .CLKFBOUT_MULT_F(63.0),
    .CLKOUT0_DIVIDE_F(50.0)
  ) vid_mmcm (
    .CLKIN1(clk200_in),
    .CLKFBIN(vid_fb_buf),
    .CLKFBOUT(vid_fb),
    .CLKOUT0(mmcm25),
    .CLKOUT0B(), .CLKOUT1(), .CLKOUT1B(), .CLKOUT2(), .CLKOUT2B(),
    .CLKOUT3(), .CLKOUT3B(), .CLKOUT4(), .CLKOUT5(), .CLKOUT6(),
    .CLKFBOUTB(),
    .LOCKED(vid_locked),
    .PWRDWN(1'b0),
    .RST(1'b0)
  );
  BUFG vid_fb_bufg (.I(vid_fb), .O(vid_fb_buf));
  BUFG pixclk_bufg (.I(mmcm25), .O(pixclk));

  // The reset for the video peripheral and the I2C master, on the
  // pixel clock, asked for by the same things as the MAC's and by the
  // video clock manager's lock. A press of the button therefore
  // re-runs the encoder's configuration, since the master drives the
  // chip's reset from its own (issue 509).
  wire vid_rst_ask = ~(reset_n & key1 & vid_locked) | brk;
  (* ASYNC_REG = "TRUE" *) reg [1:0] pix_rst_sync = 2'b11;
  always @(posedge pixclk) pix_rst_sync <= {pix_rst_sync[0], vid_rst_ask};
  wire pix_rst = pix_rst_sync[1];

  // The core's side of the crossing, declared here because the board
  // above drives it and the FIFOs below carry it.
  wire [34:0] vaw_data, var_data, qaw_data, qar_data;
  wire [35:0] vw_data, qw_data;
  wire [1:0] vb_data, qb_data;
  wire [33:0] vr_data, qr_data;
  wire vaw_valid, vaw_ready, var_valid, var_ready, vw_valid, vw_ready;
  wire vb_valid, vb_ready, vr_valid, vr_ready;
  wire qaw_valid, qaw_ready, qar_valid, qar_ready, qw_valid, qw_ready;
  wire qb_valid, qb_ready, qr_valid, qr_ready;

  // The three channels that carry a request, from the core's clock to
  // the pixel clock, and the two that carry an answer, back.
  chan_cdc #(.W(35), .AW(3)) vaw_cdc (
    .wr_clk(clk), .wr_data(vaw_data), .wr_valid(vaw_valid),
    .wr_ready(vaw_ready),
    .rd_clk(pixclk), .rd_data(qaw_data), .rd_valid(qaw_valid),
    .rd_ready(qaw_ready)
  );
  chan_cdc #(.W(35), .AW(3)) var_cdc (
    .wr_clk(clk), .wr_data(var_data), .wr_valid(var_valid),
    .wr_ready(var_ready),
    .rd_clk(pixclk), .rd_data(qar_data), .rd_valid(qar_valid),
    .rd_ready(qar_ready)
  );
  chan_cdc #(.W(36), .AW(3)) vw_cdc (
    .wr_clk(clk), .wr_data(vw_data), .wr_valid(vw_valid),
    .wr_ready(vw_ready),
    .rd_clk(pixclk), .rd_data(qw_data), .rd_valid(qw_valid),
    .rd_ready(qw_ready)
  );
  chan_cdc #(.W(2), .AW(3)) vb_cdc (
    .wr_clk(pixclk), .wr_data(qb_data), .wr_valid(qb_valid),
    .wr_ready(qb_ready),
    .rd_clk(clk), .rd_data(vb_data), .rd_valid(vb_valid),
    .rd_ready(vb_ready)
  );
  chan_cdc #(.W(34), .AW(3)) vr_cdc (
    .wr_clk(pixclk), .wr_data(qr_data), .wr_valid(qr_valid),
    .wr_ready(qr_ready),
    .rd_clk(clk), .rd_data(vr_data), .rd_valid(vr_valid),
    .rd_ready(vr_ready)
  );

  hdmi_video video (
    .clk(pixclk),
    .rst(pix_rst),
    .bus_aw_data(qaw_data), .bus_aw_valid(qaw_valid), .bus_aw_ready(qaw_ready),
    .bus_ar_data(qar_data), .bus_ar_valid(qar_valid), .bus_ar_ready(qar_ready),
    .bus_w_data(qw_data), .bus_w_valid(qw_valid), .bus_w_ready(qw_ready),
    .bus_b_data(qb_data), .bus_b_valid(qb_valid), .bus_b_ready(qb_ready),
    .bus_r_data(qr_data), .bus_r_valid(qr_valid), .bus_r_ready(qr_ready),
    .rgb(hdmi_d), .hsync(hdmi_hs), .vsync(hdmi_vs), .de(hdmi_de)
  );
  // The chip's clock is the pixel clock turned over, so the chip's
  // rising edge falls in the middle of each pixel.
  ODDR #(
    .DDR_CLK_EDGE("SAME_EDGE"), .INIT(1'b0), .SRTYPE("SYNC")
  ) clk_oddr (
    .Q(hdmi_clk), .C(pixclk), .CE(1'b1),
    .D1(1'b0), .D2(1'b1), .R(1'b0), .S(1'b0)
  );

  wire scl_low, sda_low, vid_done, vid_failed;
  hdmi_i2c master (
    .clk(pixclk),
    .rst(pix_rst),
    .sda_in(hdmi_sda),
    .nreset(hdmi_nreset), .scl_low(scl_low), .sda_low(sda_low),
    .done(vid_done), .failed(vid_failed)
  );
  assign hdmi_scl = scl_low ? 1'b0 : 1'bz;
  assign hdmi_sda = sda_low ? 1'b0 : 1'bz;
  assign hdmi_nreset_alt = hdmi_nreset;

  // --------------------------------------------------------------
  // The lights.
  // --------------------------------------------------------------

  // A frame's last byte in or out stretches its light for about a
  // fifth of a second. Both counters run on their own clock and only
  // their zero test leaves it, which is a single bit sampled by a
  // light nobody times.
  reg [24:0] seen_rx = 0, seen_tx = 0;
  always @(posedge rx_clk)
    seen_rx <= (rx_valid && rx_ready && rx_data[0]) ? ~25'd0
             : (seen_rx != 0 ? seen_rx - 1 : 0);
  always @(posedge clk125)
    seen_tx <= tx_en ? ~25'd0 : (seen_tx != 0 ? seen_tx - 1 : 0);

  // The heartbeat: bit 25 of a counter at 100 MHz toggles three times
  // a second.
  reg [25:0] beat = 0;
  always @(posedge clk) beat <= beat + 1;

  assign led1 = ~(pwm_pins[0] | halt);
  assign led2 = (seen_rx == 0) && (seen_tx == 0);
  assign led3 = ~calib;
  assign led4 = ~beat[25];
endmodule
