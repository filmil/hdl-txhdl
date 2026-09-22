// SPDX-License-Identifier: Apache-2.0
//! The whole of the design that goes on the board, as one lowered unit.
//!
//! The core, its tracker, a router, and six peripherals, each behind a
//! tracker of its own or, for the serial port and the interrupt
//! controller, an AXI-Lite bridge: the data memory at `0x1000`, the
//! interrupt controller, the timer and the software interrupt, the 64
//! KiB from `0x0200_0000`; the serial port at `0x3000`, the board's DDR3
//! memory from `0x4000_0000` to the end of the first two gigabytes,
//! the remote peripheral at `0x3300`, whose behaviour is a program on
//! another machine reached as frames on the Ethernet port,
//! and the platform-level interrupt controller at `0x0c00_0000`, where
//! RISC-V machines put it. The controller's source 1 is the serial
//! port's receive interrupt and its source 2 the board's `irq` input,
//! and its line is the core's external interrupt. `run.rs` wires
//! the same parts for a simulation, with a Rust `join` of their runs;
//! this is that wiring written as a unit of units, so `#[lower]` makes
//! its netlist, one module holding the rest, and the only thing a board
//! top has to do by hand is the clocks, the reset and the pins.
//!
//! The DDR3 controller is a foreign module inside it, so the netlist
//! names UberDDR3's wrapper and does not write it, and the memory's
//! pins, the data and strobe pads among them, are this unit's own
//! ports. `MICRON_SIM` and `BIST` are the memory controller's, and
//! `DIV` is the serial port's clock divider.
use crate::core::{Vreteno, Writeback};
use crate::dmem::Dmem;
use crate::rom::Rom;
use crate::timer::Timer;
use crate::uart::Uart;
use ddr3::Ddr3Per;
use txhdl::comp::{
    chan, join2, signal, DefaultClock, In, Out, Pad, Rx, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};
use txhdl_parts::bus::arbiter::Arbiter2;
use txhdl_parts::bus::axi::{
    Answer, Ar, Aw, AxiHost, AxiPer, Done, Grant, Issue, PerReq, B, R, W,
};
use txhdl_parts::bus::axi_lite::{
    LiteAr, LiteAw, LiteB, LiteBridge1, LiteBridge4, LiteR, LiteW,
};
use txhdl_parts::bus::axi_pins::{AxiPins, AxiPinsIn, AxiPinsOut};
use txhdl_parts::bus::router::Router6;
use txhdl_parts::eth::EthByte;
use txhdl_parts::plic::Plic2;
use txhdl_parts::pwm::Pwm;
use txhdl_parts::remote::eth::RemoteLink;
use txhdl_parts::remote::{Answer as RemoteAnswer, Ask, Remote};

/// Which device this board answers to on the wire. Every frame the
/// remote peripheral sends carries it, and a program answers each
/// device by the number it was asked under, so a wire with two of
/// these boards on it gives them a number each.
pub const REMOTE_DEV: usize = 1;

/// How long the remote peripheral waits for its program before it
/// answers the bus `SlvErr` itself: one second, at the board's 100 MHz.
/// A round trip is two frames, a program on another machine, and
/// whatever the network between them adds. Twenty milliseconds, the
/// first value, was a budget for a program on the board's own network
/// and ruled out a program anywhere else: measured on September 22,
/// 2026, a program 175 ms away by round trip answered every
/// transaction and the core read zero, the bus having refused first
/// (issue 397). One second is five times that path with room for a
/// worse one, and it is what a device that has gone away costs the
/// bus per access, which a person waits out and the serial watcher
/// outlasts. The count is cycles of the board's clock; a board on
/// another clock states its own.
pub const REMOTE_WAIT: usize = 100_000_000;

// begin{map}
/// The address map: each peripheral's base and the bits of an address
/// that must equal it. The first three are a page each; the memory is
/// the quarter of the address space from `0x4000_0000`, and the
/// interrupt controller the 64 MiB from `0x0c00_0000`.
pub type BoardRouter = Router6<
    32,
    32,
    4,
    4,
    0x1000,
    0xffff_f000,
    0x0200_0000,
    0xffff_0000,
    0x3000,
    0xffff_f000,
    0x4000_0000,
    0xc000_0000,
    0x0c00_0000,
    0xfc00_0000,
    0x0000_0000,
    0xffff_f000,
>;
// end{map}

// begin{board}
/// The board's design.
#[derive(Trace, Default)]
pub struct Board<const DIV: u32, const MICRON_SIM: usize, const BIST: usize> {
    pub cpu: Vreteno<2>,
    pub host: AxiHost<32, 32, 4, 2, 4>,
    /// The second host: Vivado's JTAG-to-AXI master, on the top beside
    /// the board's, reached over the cable that programs the part, so
    /// that memory and every peripheral can be read and written whatever
    /// the core is doing (issue 241). Its pins arrive as the board's
    /// own, and this joins them to the link's channels; a top with no
    /// master ties them off.
    pub jtag: AxiPins<32, 32, 4, 2>,
    /// The two hosts onto one link. The peripheral side carries four
    /// bits of identifier, two for the hosts' own and two for the port,
    /// which is room for four hosts before anything widens again: a
    /// direct memory access engine is the next (issue 151).
    pub arb: Arbiter2<32, 32, 4, 2, 4, 0>,
    pub router: BoardRouter,
    pub pdmem: AxiPer<32, 32, 4, 4>,
    pub ptimer: AxiPer<32, 32, 4, 4>,
    // begin{vslot}
    /// Four small peripherals share the page at `0x3000`: the serial
    /// port at `0x3000`, the pulse width modulator at `0x3100`,
    /// whatever the board hangs on the third slot at `0x3200`, and the
    /// remote peripheral at `0x3300`, each a sixteenth of the page.
    /// The router's ports go to memories and to the bus's own
    /// peripherals, and a
    /// peripheral of six registers does not want one of its own.
    ///
    /// The third slot leaves this unit as ports rather than reaching a
    /// field, because what sits there runs on a clock of its own: on
    /// the board it is the video peripheral on the pixel clock, and
    /// the crossing between the two is the board top's business. A
    /// design with nothing there ties the slot off, and a read of it
    /// answers when the tie-off does.
    pub puart: LiteBridge4<
        32,
        32,
        4,
        4,
        0x3000,
        0xffff_ff00,
        0x3100,
        0xffff_ff00,
        0x3200,
        0xffff_ff00,
        0x3300,
        0xffff_ff00,
    >,
    // end{vslot}
    pub pddr3: AxiPer<32, 32, 4, 4>,
    pub pplic: LiteBridge1<32, 32, 4, 4, 0x0c00_0000, 0xfc00_0000>,
    /// The boot memory on the bus, at address zero, readable and not
    /// writable: the same words the core fetches from inside itself,
    /// so a load can read a constant beside the code (#268).
    pub prom: AxiPer<32, 32, 4, 4>,
    pub rom: Rom<4>,
    pub dmem: Dmem<4>,
    pub timer: Timer<4>,
    pub uart: Uart<DIV>,
    pub pwm: Pwm,
    pub ddr3: Ddr3Per<MICRON_SIM, BIST>,
    // begin{remote}
    /// The peripheral at `0x3300`, whose behaviour is a program on
    /// another machine, and the link that puts its transactions on the
    /// Ethernet port as frames. Both run on the core's clock; the
    /// crossing to the port's two clocks is the board top's business,
    /// as the video slot's is, and it carries a byte and a last bit,
    /// which is what the MAC speaks.
    pub remote: Remote<REMOTE_WAIT>,
    pub link: RemoteLink<REMOTE_DEV>,
    // end{remote}
    /// Both sources ask while their line is high.
    pub plic: Plic2<0>,
}
// end{board}

// begin{ports}
/// The board's inputs: the resets, the interrupt, the serial line, the
/// memory's clocks, the video slot's answers, the frames arriving, and
/// the JTAG master's pins, named as AXI4 names them under `jtag_`.
pub struct BoardIn {
    pub rst: In<Bit>,
    pub irq: In<Bit>,
    pub rx: In<Bit>,
    pub ddr3_clk: In<Bit>,
    pub ref_clk: In<Bit>,
    pub ddr3_clk_90: In<Bit>,
    pub ddr3_rst_n: In<Bit>,
    pub vb: Rx<LiteB>,
    pub vr: Rx<LiteR<32>>,
    pub net_rx: Rx<EthByte>,
    pub jtag_awid: In<U<2>>,
    pub jtag_awaddr: In<U<32>>,
    pub jtag_awlen: In<U<8>>,
    pub jtag_awsize: In<U<3>>,
    pub jtag_awburst: In<U<2>>,
    pub jtag_awlock: In<Bit>,
    pub jtag_awcache: In<U<4>>,
    pub jtag_awprot: In<U<3>>,
    pub jtag_awvalid: In<Bit>,
    pub jtag_wdata: In<U<32>>,
    pub jtag_wstrb: In<U<4>>,
    pub jtag_wlast: In<Bit>,
    pub jtag_wvalid: In<Bit>,
    pub jtag_bready: In<Bit>,
    pub jtag_arid: In<U<2>>,
    pub jtag_araddr: In<U<32>>,
    pub jtag_arlen: In<U<8>>,
    pub jtag_arsize: In<U<3>>,
    pub jtag_arburst: In<U<2>>,
    pub jtag_arlock: In<Bit>,
    pub jtag_arcache: In<U<4>>,
    pub jtag_arprot: In<U<3>>,
    pub jtag_arvalid: In<Bit>,
    pub jtag_rready: In<Bit>,
}

/// The board's outputs: the halt and the serial line, the modulator,
/// the memory's pins, the video slot's requests, the frames leaving,
/// and the JTAG master's answers.
pub struct BoardOut {
    pub halt: Out<Bit>,
    pub tx: Out<Bit>,
    pub pwm_pins: Out<U<4>>,
    pub calib: Out<Bit>,
    pub ck_p: Out<Bit>,
    pub ck_n: Out<Bit>,
    pub mem_rst_n: Out<Bit>,
    pub cke: Out<Bit>,
    pub cs_n: Out<Bit>,
    pub ras_n: Out<Bit>,
    pub cas_n: Out<Bit>,
    pub we_n: Out<Bit>,
    pub row: Out<U<15>>,
    pub bank: Out<U<3>>,
    pub dm: Out<U<4>>,
    pub odt: Out<Bit>,
    pub dq: Pad<U<32>>,
    pub dqs: Pad<U<4>>,
    pub dqs_n: Pad<U<4>>,
    pub vaw: Tx<LiteAw<32>>,
    pub var: Tx<LiteAr<32>>,
    pub vw: Tx<LiteW<32, 4>>,
    pub net_tx: Tx<EthByte>,
    pub jtag_awready: Out<Bit>,
    pub jtag_wready: Out<Bit>,
    pub jtag_bid: Out<U<2>>,
    pub jtag_bresp: Out<U<2>>,
    pub jtag_bvalid: Out<Bit>,
    pub jtag_arready: Out<Bit>,
    pub jtag_rid: Out<U<2>>,
    pub jtag_rdata: Out<U<32>>,
    pub jtag_rresp: Out<U<2>>,
    pub jtag_rlast: Out<Bit>,
    pub jtag_rvalid: Out<Bit>,
}
// end{ports}

#[lower]
impl<const DIV: u32, const MICRON_SIM: usize, const BIST: usize> Unit
    for Board<DIV, MICRON_SIM, BIST>
{
    async fn run(
        &mut self,
        BoardIn {
            rst,
            irq,
            rx,
            ddr3_clk,
            ref_clk,
            ddr3_clk_90,
            ddr3_rst_n,
            vb,
            vr,
            net_rx,
            jtag_awid,
            jtag_awaddr,
            jtag_awlen,
            jtag_awsize,
            jtag_awburst,
            jtag_awlock,
            jtag_awcache,
            jtag_awprot,
            jtag_awvalid,
            jtag_wdata,
            jtag_wstrb,
            jtag_wlast,
            jtag_wvalid,
            jtag_bready,
            jtag_arid,
            jtag_araddr,
            jtag_arlen,
            jtag_arsize,
            jtag_arburst,
            jtag_arlock,
            jtag_arcache,
            jtag_arprot,
            jtag_arvalid,
            jtag_rready,
        }: BoardIn,
        BoardOut {
            halt,
            tx,
            pwm_pins,
            calib,
            ck_p,
            ck_n,
            mem_rst_n,
            cke,
            cs_n,
            ras_n,
            cas_n,
            we_n,
            row,
            bank,
            dm,
            odt,
            dq,
            dqs,
            dqs_n,
            vaw,
            var,
            vw,
            net_tx,
            jtag_awready,
            jtag_wready,
            jtag_bid,
            jtag_bresp,
            jtag_bvalid,
            jtag_arready,
            jtag_rid,
            jtag_rdata,
            jtag_rresp,
            jtag_rlast,
            jtag_rvalid,
        }: BoardOut,
    ) {
        // The reset, read by the core, the timer and the serial port.
        let rst_timer = rst.clone();
        let rst_uart = rst.clone();
        let rst_plic = rst.clone();
        // The core and its tracker.
        let (issue_tx, issue_rx) = chan::<Issue<32>, DefaultClock>();
        let (wbeat_tx, wbeat_rx) = chan::<W<32, 4>, DefaultClock>();
        let (release_tx, release_rx) = chan::<Grant<2>, DefaultClock>();
        let (grant_tx, grant_rx) = chan::<Grant<2>, DefaultClock>();
        let (done_tx, done_rx) = chan::<Done<2>, DefaultClock>();
        let (rdata_tx, rdata_rx) = chan::<R<32, 2>, DefaultClock>();
        let (instr_o, _instr_i) = signal::<U<32>, DefaultClock>();
        let (retire_o, _retire_i) = signal::<Writeback, DefaultClock>();
        let (tirq_o, tirq_i) = signal::<Bit, DefaultClock>();
        // The software interrupt the controller raises for a program.
        let (sirq_o, sirq_i) = signal::<Bit, DefaultClock>();
        let (uirq_o, uirq_i) = signal::<Bit, DefaultClock>();
        let (eirq_o, eirq_i) = signal::<Bit, DefaultClock>();
        // The tracker and the router.
        let (aw_tx, aw_rx) = chan::<Aw<32, 2>, DefaultClock>();
        let (ar_tx, ar_rx) = chan::<Ar<32, 2>, DefaultClock>();
        let (w_tx, w_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b_tx, b_rx) = chan::<B<2>, DefaultClock>();
        let (r_tx, r_rx) = chan::<R<32, 2>, DefaultClock>();
        // The JTAG master's pins and the arbiter, under the hosts' own
        // identifiers; and the arbiter and the router, under the wider.
        let (jaw_tx, jaw_rx) = chan::<Aw<32, 2>, DefaultClock>();
        let (jar_tx, jar_rx) = chan::<Ar<32, 2>, DefaultClock>();
        let (jw_tx, jw_rx) = chan::<W<32, 4>, DefaultClock>();
        let (jb_tx, jb_rx) = chan::<B<2>, DefaultClock>();
        let (jr_tx, jr_rx) = chan::<R<32, 2>, DefaultClock>();
        let (xaw_tx, xaw_rx) = chan::<Aw<32, 4>, DefaultClock>();
        let (xar_tx, xar_rx) = chan::<Ar<32, 4>, DefaultClock>();
        let (xw_tx, xw_rx) = chan::<W<32, 4>, DefaultClock>();
        let (xb_tx, xb_rx) = chan::<B<4>, DefaultClock>();
        let (xr_tx, xr_rx) = chan::<R<32, 4>, DefaultClock>();
        // The router and each peripheral's tracker.
        let (aw0_tx, aw0_rx) = chan::<Aw<32, 4>, DefaultClock>();
        let (ar0_tx, ar0_rx) = chan::<Ar<32, 4>, DefaultClock>();
        let (w0_tx, w0_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b0_tx, b0_rx) = chan::<B<4>, DefaultClock>();
        let (r0_tx, r0_rx) = chan::<R<32, 4>, DefaultClock>();
        let (aw1_tx, aw1_rx) = chan::<Aw<32, 4>, DefaultClock>();
        let (ar1_tx, ar1_rx) = chan::<Ar<32, 4>, DefaultClock>();
        let (w1_tx, w1_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b1_tx, b1_rx) = chan::<B<4>, DefaultClock>();
        let (r1_tx, r1_rx) = chan::<R<32, 4>, DefaultClock>();
        let (aw2_tx, aw2_rx) = chan::<Aw<32, 4>, DefaultClock>();
        let (ar2_tx, ar2_rx) = chan::<Ar<32, 4>, DefaultClock>();
        let (w2_tx, w2_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b2_tx, b2_rx) = chan::<B<4>, DefaultClock>();
        let (r2_tx, r2_rx) = chan::<R<32, 4>, DefaultClock>();
        let (aw3_tx, aw3_rx) = chan::<Aw<32, 4>, DefaultClock>();
        let (ar3_tx, ar3_rx) = chan::<Ar<32, 4>, DefaultClock>();
        let (w3_tx, w3_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b3_tx, b3_rx) = chan::<B<4>, DefaultClock>();
        let (r3_tx, r3_rx) = chan::<R<32, 4>, DefaultClock>();
        let (aw4_tx, aw4_rx) = chan::<Aw<32, 4>, DefaultClock>();
        let (ar4_tx, ar4_rx) = chan::<Ar<32, 4>, DefaultClock>();
        let (w4_tx, w4_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b4_tx, b4_rx) = chan::<B<4>, DefaultClock>();
        let (r4_tx, r4_rx) = chan::<R<32, 4>, DefaultClock>();
        let (aw5_tx, aw5_rx) = chan::<Aw<32, 4>, DefaultClock>();
        let (ar5_tx, ar5_rx) = chan::<Ar<32, 4>, DefaultClock>();
        let (w5_tx, w5_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b5_tx, b5_rx) = chan::<B<4>, DefaultClock>();
        let (r5_tx, r5_rx) = chan::<R<32, 4>, DefaultClock>();
        // The boot memory's tracker and the memory.
        let (req5_tx, req5_rx) = chan::<PerReq<32, 4>, DefaultClock>();
        let (wd5_tx, wd5_rx) = chan::<W<32, 4>, DefaultClock>();
        let (ans5_tx, ans5_rx) = chan::<Answer<4>, DefaultClock>();
        let (rb5_tx, rb5_rx) = chan::<R<32, 4>, DefaultClock>();
        // Each peripheral's tracker and the peripheral.
        let (req0_tx, req0_rx) = chan::<PerReq<32, 4>, DefaultClock>();
        let (wd0_tx, wd0_rx) = chan::<W<32, 4>, DefaultClock>();
        let (ans0_tx, ans0_rx) = chan::<Answer<4>, DefaultClock>();
        let (rb0_tx, rb0_rx) = chan::<R<32, 4>, DefaultClock>();
        let (req1_tx, req1_rx) = chan::<PerReq<32, 4>, DefaultClock>();
        let (wd1_tx, wd1_rx) = chan::<W<32, 4>, DefaultClock>();
        let (ans1_tx, ans1_rx) = chan::<Answer<4>, DefaultClock>();
        let (rb1_tx, rb1_rx) = chan::<R<32, 4>, DefaultClock>();
        // The serial port speaks AXI-Lite, behind its bridge.
        let (law_tx, law_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (lar_tx, lar_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (lw_tx, lw_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (lb_tx, lb_rx) = chan::<LiteB, DefaultClock>();
        let (lr_tx, lr_rx) = chan::<LiteR<32>, DefaultClock>();
        // The modulator's side of the same bridge.
        let (paw_pwm_tx, paw_pwm_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (par_pwm_tx, par_pwm_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (pw_pwm_tx, pw_pwm_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (pb_pwm_tx, pb_pwm_rx) = chan::<LiteB, DefaultClock>();
        let (pr_pwm_tx, pr_pwm_rx) = chan::<LiteR<32>, DefaultClock>();
        // The remote peripheral's side of the same bridge, and the two
        // channels between it and the link that makes the frames.
        let (paw_rem_tx, paw_rem_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (par_rem_tx, par_rem_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (pw_rem_tx, pw_rem_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (pb_rem_tx, pb_rem_rx) = chan::<LiteB, DefaultClock>();
        let (pr_rem_tx, pr_rem_rx) = chan::<LiteR<32>, DefaultClock>();
        let (ask_tx, ask_rx) = chan::<Ask, DefaultClock>();
        let (ans_tx, ans_rx) = chan::<RemoteAnswer, DefaultClock>();
        let (req3_tx, req3_rx) = chan::<PerReq<32, 4>, DefaultClock>();
        let (wd3_tx, wd3_rx) = chan::<W<32, 4>, DefaultClock>();
        let (ans3_tx, ans3_rx) = chan::<Answer<4>, DefaultClock>();
        let (rb3_tx, rb3_rx) = chan::<R<32, 4>, DefaultClock>();
        // The interrupt controller speaks AXI-Lite too, behind a
        // bridge of its own.
        let (paw_tx, paw_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (par_tx, par_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (pw_tx, pw_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (pb_tx, pb_rx) = chan::<LiteB, DefaultClock>();
        let (pr_tx, pr_rx) = chan::<LiteR<32>, DefaultClock>();
        // The timer first, since the core reads its line in the same
        // step, and the memory controller before the bridge inside its
        // own unit, for the same reason. The serial port before the
        // interrupt controller, and the controller before the core,
        // for the same reason again.
        join2(
            join2(
                join2(
                    self.timer.run(
                        (rst_timer, req1_rx, wd1_rx),
                        (ans1_tx, rb1_tx, tirq_o, sirq_o),
                    ),
                    join2(
                        join2(
                            self.uart.run(
                                (rst_uart, rx, law_rx, lar_rx, lw_rx),
                                (lb_tx, lr_tx, tx, uirq_o),
                            ),
                            self.pwm.run(
                                (paw_pwm_rx, par_pwm_rx, pw_pwm_rx),
                                (pb_pwm_tx, pr_pwm_tx, pwm_pins),
                            ),
                        ),
                        self.plic.run(
                            (rst_plic, uirq_i, irq, paw_rx, par_rx, pw_rx),
                            (pb_tx, pr_tx, eirq_o),
                        ),
                    ),
                ),
                join2(
                    self.dmem.run((req0_rx, wd0_rx), (ans0_tx, rb0_tx)),
                    self.cpu.run(
                        (
                            rst, eirq_i, tirq_i, sirq_i, rdata_rx, done_rx,
                            grant_rx,
                        ),
                        (
                            halt, instr_o, retire_o, issue_tx, wbeat_tx,
                            release_tx,
                        ),
                    ),
                ),
            ),
            join2(
                join2(
                    join2(
                        join2(
                            self.host.run(
                                (issue_rx, wbeat_rx, b_rx, r_rx, release_rx),
                                (
                                    aw_tx, ar_tx, w_tx, grant_tx, done_tx,
                                    rdata_tx,
                                ),
                            ),
                            self.arb.run(
                                (
                                    aw_rx, ar_rx, w_rx, jaw_rx, jar_rx, jw_rx,
                                    xb_rx, xr_rx,
                                ),
                                (
                                    xaw_tx, xar_tx, xw_tx, b_tx, r_tx, jb_tx,
                                    jr_tx,
                                ),
                            ),
                        ),
                        self.jtag.run(
                            AxiPinsIn {
                                awid: jtag_awid,
                                awaddr: jtag_awaddr,
                                awlen: jtag_awlen,
                                awsize: jtag_awsize,
                                awburst: jtag_awburst,
                                awlock: jtag_awlock,
                                awcache: jtag_awcache,
                                awprot: jtag_awprot,
                                awvalid: jtag_awvalid,
                                wdata: jtag_wdata,
                                wstrb: jtag_wstrb,
                                wlast: jtag_wlast,
                                wvalid: jtag_wvalid,
                                bready: jtag_bready,
                                arid: jtag_arid,
                                araddr: jtag_araddr,
                                arlen: jtag_arlen,
                                arsize: jtag_arsize,
                                arburst: jtag_arburst,
                                arlock: jtag_arlock,
                                arcache: jtag_arcache,
                                arprot: jtag_arprot,
                                arvalid: jtag_arvalid,
                                rready: jtag_rready,
                                b: jb_rx,
                                r: jr_rx,
                            },
                            AxiPinsOut {
                                aw: jaw_tx,
                                ar: jar_tx,
                                w: jw_tx,
                                awready: jtag_awready,
                                wready: jtag_wready,
                                bid: jtag_bid,
                                bresp: jtag_bresp,
                                bvalid: jtag_bvalid,
                                arready: jtag_arready,
                                rid: jtag_rid,
                                rdata: jtag_rdata,
                                rresp: jtag_rresp,
                                rlast: jtag_rlast,
                                rvalid: jtag_rvalid,
                            },
                        ),
                    ),
                    self.router.run(
                        (
                            xaw_rx, xar_rx, xw_rx, b0_rx, r0_rx, b1_rx, r1_rx,
                            b2_rx, r2_rx, b3_rx, r3_rx, b4_rx, r4_rx, b5_rx,
                            r5_rx,
                        ),
                        (
                            aw0_tx, ar0_tx, w0_tx, aw1_tx, ar1_tx, w1_tx,
                            aw2_tx, ar2_tx, w2_tx, aw3_tx, ar3_tx, w3_tx,
                            aw4_tx, ar4_tx, w4_tx, aw5_tx, ar5_tx, w5_tx,
                            xb_tx, xr_tx,
                        ),
                    ),
                ),
                join2(
                    join2(
                        join2(
                            self.pdmem.run(
                                (aw0_rx, ar0_rx, w0_rx, ans0_rx, rb0_rx),
                                (req0_tx, wd0_tx, b0_tx, r0_tx),
                            ),
                            self.ptimer.run(
                                (aw1_rx, ar1_rx, w1_rx, ans1_rx, rb1_rx),
                                (req1_tx, wd1_tx, b1_tx, r1_tx),
                            ),
                        ),
                        join2(
                            self.puart.run(
                                (
                                    aw2_rx, ar2_rx, w2_rx, lb_rx, lr_rx,
                                    pb_pwm_rx, pr_pwm_rx, vb, vr, pb_rem_rx,
                                    pr_rem_rx,
                                ),
                                (
                                    law_tx, lar_tx, lw_tx, paw_pwm_tx,
                                    par_pwm_tx, pw_pwm_tx, vaw, var, vw,
                                    paw_rem_tx, par_rem_tx, pw_rem_tx, b2_tx,
                                    r2_tx,
                                ),
                            ),
                            join2(
                                self.pddr3.run(
                                    (aw3_rx, ar3_rx, w3_rx, ans3_rx, rb3_rx),
                                    (req3_tx, wd3_tx, b3_tx, r3_tx),
                                ),
                                join2(
                                    join2(
                                        self.pplic.run(
                                            (
                                                aw4_rx, ar4_rx, w4_rx, pb_rx,
                                                pr_rx,
                                            ),
                                            (
                                                paw_tx, par_tx, pw_tx, b4_tx,
                                                r4_tx,
                                            ),
                                        ),
                                        join2(
                                            self.prom.run(
                                                (
                                                    aw5_rx, ar5_rx, w5_rx,
                                                    ans5_rx, rb5_rx,
                                                ),
                                                (req5_tx, wd5_tx, b5_tx, r5_tx),
                                            ),
                                            self.rom.run(
                                                (req5_rx, wd5_rx),
                                                (ans5_tx, rb5_tx),
                                            ),
                                        ),
                                    ),
                                    // The peripheral before the link, so
                                    // a transaction and the first byte of
                                    // its frame are one step apart rather
                                    // than two.
                                    join2(
                                        self.remote.run(
                                            (
                                                paw_rem_rx, par_rem_rx,
                                                pw_rem_rx, ans_rx,
                                            ),
                                            (pb_rem_tx, pr_rem_tx, ask_tx),
                                        ),
                                        self.link.run(
                                            (ask_rx, net_rx),
                                            (ans_tx, net_tx),
                                        ),
                                    ),
                                ),
                            ),
                        ),
                    ),
                    self.ddr3.run(
                        (
                            req3_rx,
                            wd3_rx,
                            ddr3_clk,
                            ref_clk,
                            ddr3_clk_90,
                            ddr3_rst_n,
                        ),
                        (
                            ans3_tx, rb3_tx, calib, ck_p, ck_n, mem_rst_n, cke,
                            cs_n, ras_n, cas_n, we_n, row, bank, dm, odt, dq,
                            dqs, dqs_n,
                        ),
                    ),
                ),
            ),
        )
        .await;
    }
}
