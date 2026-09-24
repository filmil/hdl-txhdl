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
//! the Ethernet port's registers at `0x3400`,
//! and the platform-level interrupt controller at `0x0c00_0000`, where
//! RISC-V machines put it. The controller's source 1 is the serial
//! port's receive interrupt, its source 2 the board's `irq` input and
//! its source 3 a frame arriving on the Ethernet port,
//! and its line is the core's external interrupt. `run.rs` wires
//! the same parts for a simulation, with a Rust `join` of their runs;
//! this is that wiring written as a unit of units, so `#[lower]` makes
//! its netlist, one module holding the rest, and the only thing a board
//! top has to do by hand is the clocks, the reset and the pins.
//!
//! The DDR3 controller is a foreign module inside it, so the netlist
//! names the controller's wrapper and does not write it, and the
//! memory's pins, the data and strobe pads among them, are this unit's
//! own ports, as are the board's clock going in and the design's clock
//! coming out, since the controller makes it. `DIV` is the serial
//! port's clock divider.
use crate::core::{Vreteno, Writeback};
use crate::debug::Dm;
use crate::dmem::Dmem;
use crate::isa;
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
    LiteAr, LiteAw, LiteB, LiteBridge1, LiteBridge5, LitePort, LiteR, LiteW,
};
use txhdl_parts::bus::axi_pins::{AxiPins, AxiPinsIn, AxiPinsOut};
use txhdl_parts::bus::router::Router7;
use txhdl_parts::eth::EthByte;
use txhdl_parts::ethslots::EthSlots;
use txhdl_parts::plic::Plic3;
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
/// interrupt controller the 64 MiB from `0x0c00_0000`; the debug
/// module has the 64 KiB from `0x1000_0000` (issue 154).
pub type BoardRouter = Router7<
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
    0x1000_0000,
    0xffff_0000,
>;
// end{map}

// begin{board}
/// The board's design.
#[derive(Trace, Default)]
pub struct Board<const DIV: u32> {
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
    /// Five small peripherals share the page at `0x3000`: the serial
    /// port at `0x3000`, the pulse width modulator at `0x3100`,
    /// whatever the board hangs on the third slot at `0x3200`, the
    /// remote peripheral at `0x3300`, and the Ethernet port's
    /// registers on the fifth slot at `0x3400`, each a sixteenth of
    /// the page. The router's ports go to memories and to the bus's
    /// own peripherals, and a
    /// peripheral of six registers does not want one of its own.
    ///
    /// The page was never the constraint and is not now. It is 4 KiB
    /// and a slot is 256 bytes, so it holds sixteen and eleven are
    /// still free; what was full was the bridge in front of it, which
    /// had four ports. So no address moves to make room for the fifth,
    /// and nothing that names one of the first four changes.
    ///
    /// The third slot leaves this unit as ports rather than reaching a
    /// field, because what sits there runs on a clock of its own: on
    /// the board it is the video peripheral on the pixel clock, and
    /// the crossing between the two is the board top's business. A
    /// design with nothing there ties the slot off, and a read of it
    /// answers when the tie-off does.
    ///
    /// The fifth is a field, because `EthSlots` runs on the bus clock
    /// like every other peripheral here and wants no crossing.
    pub puart: LiteBridge5<
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
        0x3400,
        0xffff_ff00,
    >,
    // end{vslot}
    pub pddr3: AxiPer<32, 32, 4, 4>,
    pub pplic: LiteBridge1<32, 32, 4, 4, 0x0c00_0000, 0xfc00_0000>,
    /// The debug module, on a router port of its own behind a bridge
    /// of its own, so the JTAG host reaches it while the core is
    /// halted (issue 154).
    pub pdm: LiteBridge1<32, 32, 4, 4, 0x1000_0000, 0xffff_0000>,
    pub dmod: Dm,
    /// The boot memory on the bus, at address zero, readable and not
    /// writable: the same words the core fetches from inside itself,
    /// so a load can read a constant beside the code (#268).
    pub prom: AxiPer<32, 32, 4, 4>,
    pub rom: Rom<4>,
    pub dmem: Dmem<4>,
    pub timer: Timer<4>,
    pub uart: Uart<DIV>,
    pub pwm: Pwm,
    pub ddr3: Ddr3Per,
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
    // begin{ethslot}
    /// The Ethernet port's registers, on the fifth slot at `0x3400`,
    /// with its four frame buffers in the memory from
    /// [`isa::ETH_BUF_BASE`](crate::isa::ETH_BUF_BASE).
    ///
    /// It answers its registers and moves no frames. The seven ports
    /// it has besides the bus face the engines that would carry the
    /// bytes, `LineFetch` and `LineStore` with an adapter between the
    /// words they move and the bytes a frame counts, and those are
    /// issue 151 and are not written. Until they are, the busy lines
    /// it reads are tied low and what it drives goes nowhere: a driver
    /// binds, reads and writes every register, and no frame arrives or
    /// leaves.
    ///
    /// That is worth having before the engines rather than after. The
    /// device tree describes a peripheral that is really there, the
    /// driver is compiled by `bazel test //...` from the first commit
    /// that enables it rather than whenever somebody next tries a
    /// board, and the register map has two implementations to
    /// disagree with each other rather than one and a document.
    pub eth: EthSlots<{ isa::ETH_BUF_BASE as usize }>,
    // end{ethslot}
    /// Three sources, each asking while its line is high: the serial
    /// port's receive interrupt, the board's own `irq` input, and the
    /// Ethernet port's arrival.
    pub plic: Plic3<0>,
}
// end{board}

// begin{ports}
/// The board's inputs: the resets, the interrupt, the serial line, the
/// board's clock and the controller's reset, the video slot's answers, the frames arriving, and
/// the JTAG master's pins, named as AXI4 names them under `jtag_`.
pub struct BoardIn {
    pub rst: In<Bit>,
    pub irq: In<Bit>,
    pub rx: In<Bit>,
    pub sys_clk: In<Bit>,
    pub sys_rst: In<Bit>,
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
    pub ui_clk: Out<Bit>,
    pub ui_rst: Out<Bit>,
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
impl<const DIV: u32> Unit for Board<DIV> {
    async fn run(
        &mut self,
        BoardIn {
            rst,
            irq,
            rx,
            sys_clk,
            sys_rst,
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
            ui_clk,
            ui_rst,
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
        // The debug module's lines to the core and back: the two
        // requests, the register access, and debug mode with the word
        // read (issue 154).
        let (haltreq_o, haltreq_i) = signal::<Bit, DefaultClock>();
        let (resumereq_o, resumereq_i) = signal::<Bit, DefaultClock>();
        let (dbg_regno_o, dbg_regno_i) = signal::<U<16>, DefaultClock>();
        let (dbg_wdata_o, dbg_wdata_i) = signal::<U<32>, DefaultClock>();
        let (dbg_we_o, dbg_we_i) = signal::<Bit, DefaultClock>();
        let (debug_o, debug_i) = signal::<Bit, DefaultClock>();
        let (dbg_rdata_o, dbg_rdata_i) = signal::<U<32>, DefaultClock>();
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
        // The fifth slot, to the Ethernet port's registers.
        let (paw_eth_tx, paw_eth_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (par_eth_tx, par_eth_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (pw_eth_tx, pw_eth_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (pb_eth_tx, pb_eth_rx) = chan::<LiteB, DefaultClock>();
        let (pr_eth_tx, pr_eth_rx) = chan::<LiteR<32>, DefaultClock>();
        // Its arrival line, which is the interrupt controller's third
        // source.
        let (eth_irq_o, eth_irq_i) = signal::<Bit, DefaultClock>();
        // What it says to the engines that would move the frames, and
        // what they would say back. There are no engines yet (issue
        // 151), so the lines it reads are driven low and nothing reads
        // what it drives: it answers its registers and no frame moves.
        // `tx_busy` low is what lets `tx_ready` read one until a
        // driver starts a transmit; nothing then clears `tx_go`, so
        // the second transmit waits, which is the truth about a port
        // with no engine behind it.
        let (_eth_tx_busy_o, eth_tx_busy_i) = signal::<Bit, DefaultClock>();
        let (_eth_rx_busy_o, eth_rx_busy_i) = signal::<Bit, DefaultClock>();
        let (_eth_rx_len_o, eth_rx_len_i) = signal::<U<16>, DefaultClock>();
        let (_eth_rx_which_o, eth_rx_which_i) = signal::<U<1>, DefaultClock>();
        let (eth_tx_base_o, _eth_tx_base_i) = signal::<U<32>, DefaultClock>();
        let (eth_tx_bytes_o, _eth_tx_bytes_i) = signal::<U<16>, DefaultClock>();
        let (eth_tx_start_o, _eth_tx_start_i) = signal::<Bit, DefaultClock>();
        let (eth_rx_base_o, _eth_rx_base_i) = signal::<U<32>, DefaultClock>();
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
        // And the debug module, on the seventh port.
        let (aw6_tx, aw6_rx) = chan::<Aw<32, 4>, DefaultClock>();
        let (ar6_tx, ar6_rx) = chan::<Ar<32, 4>, DefaultClock>();
        let (w6_tx, w6_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b6_tx, b6_rx) = chan::<B<4>, DefaultClock>();
        let (r6_tx, r6_rx) = chan::<R<32, 4>, DefaultClock>();
        let (daw_tx, daw_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (dar_tx, dar_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (dw_tx, dw_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (db_tx, db_rx) = chan::<LiteB, DefaultClock>();
        let (dr_tx, dr_rx) = chan::<LiteR<32>, DefaultClock>();
        // The timer first, since the core reads its line in the same
        // step, and the memory controller before the bridge inside its
        // own unit, for the same reason. The serial port before the
        // interrupt controller, and the controller before the core,
        // for the same reason again.
        join2(
            join2(
                join2(
                    join2(
                        // The debug module before the core, whose
                        // requests the core reads in the same step.
                        join2(
                            self.dmod.run(
                                LitePort {
                                    aw: daw_rx,
                                    ar: dar_rx,
                                    w: dw_rx,
                                    b: db_tx,
                                    r: dr_tx,
                                },
                                (
                                    debug_i,
                                    dbg_rdata_i,
                                    haltreq_o,
                                    resumereq_o,
                                    dbg_regno_o,
                                    dbg_wdata_o,
                                    dbg_we_o,
                                ),
                            ),
                            self.pdm.run(
                                (aw6_rx, ar6_rx, w6_rx, db_rx, dr_rx),
                                (daw_tx, dar_tx, dw_tx, b6_tx, r6_tx),
                            ),
                        ),
                        self.timer.run(
                            (rst_timer, req1_rx, wd1_rx),
                            (ans1_tx, rb1_tx, tirq_o, sirq_o),
                        ),
                    ),
                    join2(
                        join2(
                            self.uart.run(
                                LitePort {
                                    aw: law_rx,
                                    ar: lar_rx,
                                    w: lw_rx,
                                    b: lb_tx,
                                    r: lr_tx,
                                },
                                (rst_uart, rx, tx, uirq_o),
                            ),
                            self.pwm.run(
                                LitePort {
                                    aw: paw_pwm_rx,
                                    ar: par_pwm_rx,
                                    w: pw_pwm_rx,
                                    b: pb_pwm_tx,
                                    r: pr_pwm_tx,
                                },
                                pwm_pins,
                            ),
                        ),
                        join2(
                            self.plic.run(
                                LitePort {
                                    aw: paw_rx,
                                    ar: par_rx,
                                    w: pw_rx,
                                    b: pb_tx,
                                    r: pr_tx,
                                },
                                (rst_plic, uirq_i, irq, eth_irq_i, eirq_o),
                            ),
                            self.eth.run(
                                LitePort {
                                    aw: paw_eth_rx,
                                    ar: par_eth_rx,
                                    w: pw_eth_rx,
                                    b: pb_eth_tx,
                                    r: pr_eth_tx,
                                },
                                (
                                    eth_tx_busy_i,
                                    eth_rx_busy_i,
                                    eth_rx_len_i,
                                    eth_rx_which_i,
                                    eth_tx_base_o,
                                    eth_tx_bytes_o,
                                    eth_tx_start_o,
                                    eth_rx_base_o,
                                    eth_irq_o,
                                ),
                            ),
                        ),
                    ),
                ),
                join2(
                    self.dmem.run((req0_rx, wd0_rx), (ans0_tx, rb0_tx)),
                    self.cpu.run(
                        (
                            rst,
                            eirq_i,
                            tirq_i,
                            sirq_i,
                            rdata_rx,
                            done_rx,
                            grant_rx,
                            haltreq_i,
                            resumereq_i,
                            dbg_regno_i,
                            dbg_wdata_i,
                            dbg_we_i,
                        ),
                        (
                            halt,
                            instr_o,
                            retire_o,
                            issue_tx,
                            wbeat_tx,
                            release_tx,
                            debug_o,
                            dbg_rdata_o,
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
                            r5_rx, b6_rx, r6_rx,
                        ),
                        (
                            aw0_tx, ar0_tx, w0_tx, aw1_tx, ar1_tx, w1_tx,
                            aw2_tx, ar2_tx, w2_tx, aw3_tx, ar3_tx, w3_tx,
                            aw4_tx, ar4_tx, w4_tx, aw5_tx, ar5_tx, w5_tx,
                            aw6_tx, ar6_tx, w6_tx, xb_tx, xr_tx,
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
                                    pr_rem_rx, pb_eth_rx, pr_eth_rx,
                                ),
                                (
                                    law_tx, lar_tx, lw_tx, paw_pwm_tx,
                                    par_pwm_tx, pw_pwm_tx, vaw, var, vw,
                                    paw_rem_tx, par_rem_tx, pw_rem_tx,
                                    paw_eth_tx, par_eth_tx, pw_eth_tx, b2_tx,
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
                        (req3_rx, wd3_rx, sys_clk, sys_rst),
                        (
                            ans3_tx, rb3_tx, calib, ui_clk, ui_rst, ck_p, ck_n,
                            mem_rst_n, cke, cs_n, ras_n, cas_n, we_n, row,
                            bank, dm, odt, dq, dqs, dqs_n,
                        ),
                    ),
                ),
            ),
        )
        .await;
    }
}
