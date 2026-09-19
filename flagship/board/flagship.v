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
//     PHY's RGMII and the crossing between the two clocks, echoing
//     every good frame. The core does not reach the port yet, so the
//     echo is what keeps the port alive and provable from the other end
//     of the cable while a peripheral for it is written.
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
//   * a PLL, times six to 1200 MHz: 100 MHz for the design and the
//     memory controller, 400 MHz for the memory, the same 400 MHz a
//     quarter cycle late, and 200 MHz for the controller's delay
//     control. This is `vreteno_board.v`'s PLL, unchanged.
//   * an MMCM, times five to 1000 MHz over eight: 125 MHz for the
//     Ethernet transmit half. This is `eth_echo.v`'s, unchanged. A
//     second MMCM inside `eth_rgmii` shifts the PHY's receive clock.
//   * an MMCM, times 63 over ten and then over 50: 25.2 MHz for the
//     pixel clock. This is `hdmi_demo.v`'s, unchanged.
//
// One thing crosses between the domains, and it is a channel rather
// than a signal. The core's third AXI-Lite slot, the page at `0x3200`,
// leaves `board` as five channel ports and reaches the video
// peripheral on the pixel clock through five `chan_cdc` FIFOs, one per
// AXI-Lite channel, each carrying its words in the direction that
// channel runs. Gray-coded pointers through two flip-flops on each
// side, which is the same crossing the Ethernet half already uses on
// this board. The five channels of AXI-Lite have no timing
// relationship to each other, so crossing each on its own is the whole
// of what the protocol asks.
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
//   led4  the heartbeat, three times a second, which says the PLL has
//         locked and this part holds a bitstream that is running.
//
// The HDMI has no light of its own; the monitor is its light.
`timescale 1ps / 1ps
module flagship (
  input sys_clk_p,
  input sys_clk_n,
  input reset_n,
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

  wire fb, fb_buf, locked;
  wire pll100, pll400, pll200, pll400_90;
  wire clk, clk400, clk200, clk400_90;
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
  wire calib;
  reg [1:0] mem_sync = 2'b00;
  reg [1:0] core_sync = 2'b11;
  always @(posedge clk) begin
    mem_sync <= {mem_sync[0], reset_n & locked};
    core_sync <= {core_sync[0], ~(reset_n & locked & calib)};
  end
  wire mem_rst_n = mem_sync[1];
  wire rst = core_sync[1];

  wire halt;
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
    // The video slot at `0x3200`, on the core's clock, crossed below.
    .vaw_data(vaw_data), .vaw_valid(vaw_valid), .vaw_ready(vaw_ready),
    .var_data(var_data), .var_valid(var_valid), .var_ready(var_ready),
    .vw_data(vw_data), .vw_valid(vw_valid), .vw_ready(vw_ready),
    .vb_data(vb_data), .vb_valid(vb_valid), .vb_ready(vb_ready),
    .vr_data(vr_data), .vr_valid(vr_valid), .vr_ready(vr_ready)
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
    .rxd(rxd), .rx_dv(rx_dv), .rx_er(rx_er),
    .rx_data(rx_data), .rx_valid(rx_valid), .rx_ready(rx_ready)
  );

  wire [8:0] tx_data;
  wire tx_valid, tx_ready;
  chan_cdc #(.W(9), .AW(4)) crossing (
    .wr_clk(rx_clk), .wr_data(rx_data), .wr_valid(rx_valid),
    .wr_ready(rx_ready),
    .rd_clk(clk125), .rd_data(tx_data), .rd_valid(tx_valid),
    .rd_ready(tx_ready)
  );

  eth_tx mac_tx (
    .clk(clk125),
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
    .aw_data(qaw_data), .aw_valid(qaw_valid), .aw_ready(qaw_ready),
    .ar_data(qar_data), .ar_valid(qar_valid), .ar_ready(qar_ready),
    .w_data(qw_data), .w_valid(qw_valid), .w_ready(qw_ready),
    .b_data(qb_data), .b_valid(qb_valid), .b_ready(qb_ready),
    .r_data(qr_data), .r_valid(qr_valid), .r_ready(qr_ready),
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
