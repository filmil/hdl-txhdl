// SPDX-License-Identifier: Apache-2.0
//! The whole of the design that goes on the board, as one lowered unit.
//!
//! The core, its tracker, a router, and five peripherals, each behind a
//! tracker of its own or, for the serial port and the interrupt
//! controller, an AXI-Lite bridge: the data memory at `0x1000`, the
//! timer at `0x2000`, the serial port at `0x3000`, the board's DDR3
//! memory from `0x4000_0000` to the end of the first two gigabytes,
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
use crate::timer::Timer;
use crate::uart::Uart;
use ddr3::Ddr3Per;
use txhdl::comp::{chan, join2, signal, DefaultClock, In, Out, Pad, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};
use txhdl_parts::bus::axi::{
    Answer, Ar, Aw, AxiHost, AxiPer, Done, Grant, Issue, PerReq, B, R, W,
};
use txhdl_parts::bus::axi_lite::{
    LiteAr, LiteAw, LiteB, LiteBridge1, LiteR, LiteW,
};
use txhdl_parts::bus::router::Router5;
use txhdl_parts::plic::Plic2;

// begin{map}
/// The address map: each peripheral's base and the bits of an address
/// that must equal it. The first three are a page each; the memory is
/// the quarter of the address space from `0x4000_0000`, and the
/// interrupt controller the 64 MiB from `0x0c00_0000`.
pub type BoardRouter = Router5<
    32,
    32,
    4,
    2,
    0x1000,
    0xffff_f000,
    0x2000,
    0xffff_f000,
    0x3000,
    0xffff_f000,
    0x4000_0000,
    0xc000_0000,
    0x0c00_0000,
    0xfc00_0000,
>;
// end{map}

// begin{board}
/// The board's design.
#[derive(Trace, Default)]
pub struct Board<const DIV: u32, const MICRON_SIM: usize, const BIST: usize> {
    pub cpu: Vreteno<2>,
    pub host: AxiHost<32, 32, 4, 2, 4>,
    pub router: BoardRouter,
    pub pdmem: AxiPer<32, 32, 4, 2>,
    pub ptimer: AxiPer<32, 32, 4, 2>,
    pub puart: LiteBridge1<32, 32, 4, 2, 0x3000, 0xffff_f000>,
    pub pddr3: AxiPer<32, 32, 4, 2>,
    pub pplic: LiteBridge1<32, 32, 4, 2, 0x0c00_0000, 0xfc00_0000>,
    pub dmem: Dmem<2>,
    pub timer: Timer<2>,
    pub uart: Uart<DIV>,
    pub ddr3: Ddr3Per<MICRON_SIM, BIST>,
    /// Both sources ask while their line is high.
    pub plic: Plic2<0>,
}
// end{board}

#[lower]
impl<const DIV: u32, const MICRON_SIM: usize, const BIST: usize> Unit
    for Board<DIV, MICRON_SIM, BIST>
{
    async fn run(
        &mut self,
        (rst, irq, rx, ddr3_clk, ref_clk, ddr3_clk_90, ddr3_rst_n): (
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
        ),
        (
            halt,
            tx,
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
        ): (
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<U<15>>,
            Out<U<3>>,
            Out<U<4>>,
            Out<Bit>,
            Pad<U<32>>,
            Pad<U<4>>,
            Pad<U<4>>,
        ),
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
        let (uirq_o, uirq_i) = signal::<Bit, DefaultClock>();
        let (eirq_o, eirq_i) = signal::<Bit, DefaultClock>();
        // The tracker and the router.
        let (aw_tx, aw_rx) = chan::<Aw<32, 2>, DefaultClock>();
        let (ar_tx, ar_rx) = chan::<Ar<32, 2>, DefaultClock>();
        let (w_tx, w_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b_tx, b_rx) = chan::<B<2>, DefaultClock>();
        let (r_tx, r_rx) = chan::<R<32, 2>, DefaultClock>();
        // The router and each peripheral's tracker.
        let (aw0_tx, aw0_rx) = chan::<Aw<32, 2>, DefaultClock>();
        let (ar0_tx, ar0_rx) = chan::<Ar<32, 2>, DefaultClock>();
        let (w0_tx, w0_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b0_tx, b0_rx) = chan::<B<2>, DefaultClock>();
        let (r0_tx, r0_rx) = chan::<R<32, 2>, DefaultClock>();
        let (aw1_tx, aw1_rx) = chan::<Aw<32, 2>, DefaultClock>();
        let (ar1_tx, ar1_rx) = chan::<Ar<32, 2>, DefaultClock>();
        let (w1_tx, w1_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b1_tx, b1_rx) = chan::<B<2>, DefaultClock>();
        let (r1_tx, r1_rx) = chan::<R<32, 2>, DefaultClock>();
        let (aw2_tx, aw2_rx) = chan::<Aw<32, 2>, DefaultClock>();
        let (ar2_tx, ar2_rx) = chan::<Ar<32, 2>, DefaultClock>();
        let (w2_tx, w2_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b2_tx, b2_rx) = chan::<B<2>, DefaultClock>();
        let (r2_tx, r2_rx) = chan::<R<32, 2>, DefaultClock>();
        let (aw3_tx, aw3_rx) = chan::<Aw<32, 2>, DefaultClock>();
        let (ar3_tx, ar3_rx) = chan::<Ar<32, 2>, DefaultClock>();
        let (w3_tx, w3_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b3_tx, b3_rx) = chan::<B<2>, DefaultClock>();
        let (r3_tx, r3_rx) = chan::<R<32, 2>, DefaultClock>();
        let (aw4_tx, aw4_rx) = chan::<Aw<32, 2>, DefaultClock>();
        let (ar4_tx, ar4_rx) = chan::<Ar<32, 2>, DefaultClock>();
        let (w4_tx, w4_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b4_tx, b4_rx) = chan::<B<2>, DefaultClock>();
        let (r4_tx, r4_rx) = chan::<R<32, 2>, DefaultClock>();
        // Each peripheral's tracker and the peripheral.
        let (req0_tx, req0_rx) = chan::<PerReq<32, 2>, DefaultClock>();
        let (wd0_tx, wd0_rx) = chan::<W<32, 4>, DefaultClock>();
        let (ans0_tx, ans0_rx) = chan::<Answer<2>, DefaultClock>();
        let (rb0_tx, rb0_rx) = chan::<R<32, 2>, DefaultClock>();
        let (req1_tx, req1_rx) = chan::<PerReq<32, 2>, DefaultClock>();
        let (wd1_tx, wd1_rx) = chan::<W<32, 4>, DefaultClock>();
        let (ans1_tx, ans1_rx) = chan::<Answer<2>, DefaultClock>();
        let (rb1_tx, rb1_rx) = chan::<R<32, 2>, DefaultClock>();
        // The serial port speaks AXI-Lite, behind its bridge.
        let (law_tx, law_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (lar_tx, lar_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (lw_tx, lw_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (lb_tx, lb_rx) = chan::<LiteB, DefaultClock>();
        let (lr_tx, lr_rx) = chan::<LiteR<32>, DefaultClock>();
        let (req3_tx, req3_rx) = chan::<PerReq<32, 2>, DefaultClock>();
        let (wd3_tx, wd3_rx) = chan::<W<32, 4>, DefaultClock>();
        let (ans3_tx, ans3_rx) = chan::<Answer<2>, DefaultClock>();
        let (rb3_tx, rb3_rx) = chan::<R<32, 2>, DefaultClock>();
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
                        (ans1_tx, rb1_tx, tirq_o),
                    ),
                    join2(
                        self.uart.run(
                            (rst_uart, rx, law_rx, lar_rx, lw_rx),
                            (lb_tx, lr_tx, tx, uirq_o),
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
                        (rst, eirq_i, tirq_i, rdata_rx, done_rx, grant_rx),
                        (
                            halt, instr_o, retire_o, issue_tx, wbeat_tx,
                            release_tx,
                        ),
                    ),
                ),
            ),
            join2(
                join2(
                    self.host.run(
                        (issue_rx, wbeat_rx, b_rx, r_rx, release_rx),
                        (aw_tx, ar_tx, w_tx, grant_tx, done_tx, rdata_tx),
                    ),
                    self.router.run(
                        (
                            aw_rx, ar_rx, w_rx, b0_rx, r0_rx, b1_rx, r1_rx,
                            b2_rx, r2_rx, b3_rx, r3_rx, b4_rx, r4_rx,
                        ),
                        (
                            aw0_tx, ar0_tx, w0_tx, aw1_tx, ar1_tx, w1_tx,
                            aw2_tx, ar2_tx, w2_tx, aw3_tx, ar3_tx, w3_tx,
                            aw4_tx, ar4_tx, w4_tx, b_tx, r_tx,
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
                                (aw2_rx, ar2_rx, w2_rx, lb_rx, lr_rx),
                                (law_tx, lar_tx, lw_tx, b2_tx, r2_tx),
                            ),
                            join2(
                                self.pddr3.run(
                                    (aw3_rx, ar3_rx, w3_rx, ans3_rx, rb3_rx),
                                    (req3_tx, wd3_tx, b3_tx, r3_tx),
                                ),
                                self.pplic.run(
                                    (aw4_rx, ar4_rx, w4_rx, pb_rx, pr_rx),
                                    (paw_tx, par_tx, pw_tx, b4_tx, r4_tx),
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
