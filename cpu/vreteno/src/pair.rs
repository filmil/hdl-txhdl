// SPDX-License-Identifier: Apache-2.0
//! Two cores running one program, and a line that says they disagreed.
//!
//! A machine that must not be quietly wrong runs two of itself and
//! compares. What makes it worth something is that both copies see
//! exactly the same inputs in the same cycles, so that any difference
//! in what they do is theirs and not their inputs'; and that the
//! comparison covers everything a core says, not a sample of it.
//!
//! `Pair` holds two `Vreteno` cores. Every input the pair is given
//! goes to both: the wires directly, and the three channels through a
//! `Tee` each, which hands a word to both cores in one cycle or to
//! neither. Every output is compared: the three channels through a
//! `Check` each, which takes a word from both cores in one cycle,
//! passes the first core's on and raises a line when the two differ,
//! and the three wires through `Watch`, which compares them every
//! cycle.
//!
//! What leaves the pair is the first core's, so a design around it
//! sees one core. What it adds is `differs`, which goes high in the
//! cycle the two cores stop agreeing and stays high until `rst`.
//!
//! `fault` is for the self-test: while it is high the second core is
//! held in reset, which is a divergence with a known cause, and every
//! part of the comparison path is exercised by it.
use txhdl::comp::{
    chan, join2, signal, Clock, DefaultClock, In, Out, Reg, Rx, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::{Done, Grant, Issue, R, W};
use txhdl_parts::redundant::{Check, Tee};

use crate::core::{Vreteno, Writeback};

// begin{watch}
/// The three wires a core drives, from both cores, and the three lines
/// the checks raise. It passes the first core's wires on, compares
/// them with the second's every cycle, since a wire has no handshake,
/// and gathers every disagreement into one line that stays raised
/// until `rst`.
#[derive(Trace, Default)]
pub struct Watch {
    /// Whether a disagreement has been seen since the last reset.
    pub differed: Reg<Bit>,
}

#[lower]
impl Unit for Watch {
    async fn run(
        &mut self,
        (rst, halt1, halt2, instr1, instr2, wb1, wb2, d1, d2, d3): (
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<U<32>>,
            In<U<32>>,
            In<Writeback>,
            In<Writeback>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
        ),
        (halt, instr, wb, differs): (
            Out<Bit>,
            Out<U<32>>,
            Out<Writeback>,
            Out<Bit>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let bad = (halt1.get() ^ halt2.get())
                | Bit::from(instr1.get() != instr2.get())
                | Bit::from(wb1.get() != wb2.get())
                | d1.get()
                | d2.get()
                | d3.get();
            with!(self <= {
                rst.get() ? differed: Bit::Zero,
                bad & !rst.get() ? differed: Bit::One,
            });
            halt.set(halt1.get());
            instr.set(instr1.get());
            wb.set(wb1.get());
            differs.set(self.differed.get() | bad);
        }
    }
}
// end{watch}

// begin{inject}
/// The second core's reset: the pair's, held while `fault` is high.
/// That is the self-test. A core held in reset a cycle longer than the
/// other is a divergence with a known cause, and every part of the
/// comparison path is exercised by it.
#[derive(Trace, Default)]
pub struct Inject {}

#[lower]
impl Unit for Inject {
    async fn run(&mut self, (rst, fault): (In<Bit>, In<Bit>), rst2: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            rst2.set(rst.get() | fault.get());
        }
    }
}
// end{inject}

// begin{state}
/// Two cores in step, with everything they say compared.
#[derive(Trace, Default)]
pub struct Pair<const IW: usize> {
    /// The core whose work leaves the pair.
    pub one: Vreteno<IW>,
    /// The core that only checks.
    pub two: Vreteno<IW>,
    /// The read answers, to both cores.
    pub tee_rdata: Tee<R<32, IW>>,
    /// The write completions, to both.
    pub tee_done: Tee<Done<IW>>,
    /// The identifiers handed out, to both.
    pub tee_grant: Tee<Grant<IW>>,
    /// The bursts they issue, compared.
    pub check_issue: Check<Issue<32>>,
    /// The write beats they send, compared.
    pub check_wbeat: Check<W<32, 4>>,
    /// The identifiers they release, compared.
    pub check_release: Check<Grant<IW>>,
    /// Their three wires, compared, and passed on.
    pub watch: Watch,
    /// The self-test's one difference.
    pub inject: Inject,
}
// end{state}

// begin{run}
#[lower]
impl<const IW: usize> Unit for Pair<IW> {
    async fn run(
        &mut self,
        (rst, irq, tirq, fault, rdata, done, grant): (
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
            Rx<R<32, IW>>,
            Rx<Done<IW>>,
            Rx<Grant<IW>>,
        ),
        (halt, instr, wb, issue, wbeat, release, differs): (
            Out<Bit>,
            Out<U<32>>,
            Out<Writeback>,
            Tx<Issue<32>>,
            Tx<W<32, 4>>,
            Tx<Grant<IW>>,
            Out<Bit>,
        ),
    ) {
        // The reset and the two lines, read by more than one child.
        let rst_one = rst.clone();
        let rst_inj = rst.clone();
        let rst_i = rst.clone();
        let rst_w = rst.clone();
        let rst_r = rst.clone();
        let irq_one = irq.clone();
        let irq2 = irq.clone();
        let tirq_two = tirq.clone();
        // What the tees hand to each core.
        let (rd1_tx, rd1_rx) = chan::<R<32, IW>, DefaultClock>();
        let (rd2_tx, rd2_rx) = chan::<R<32, IW>, DefaultClock>();
        let (dn1_tx, dn1_rx) = chan::<Done<IW>, DefaultClock>();
        let (dn2_tx, dn2_rx) = chan::<Done<IW>, DefaultClock>();
        let (gr1_tx, gr1_rx) = chan::<Grant<IW>, DefaultClock>();
        let (gr2_tx, gr2_rx) = chan::<Grant<IW>, DefaultClock>();
        // What each core says, on its way to the checks.
        let (is1_tx, is1_rx) = chan::<Issue<32>, DefaultClock>();
        let (is2_tx, is2_rx) = chan::<Issue<32>, DefaultClock>();
        let (beat1_tx, beat1_rx) = chan::<W<32, 4>, DefaultClock>();
        let (beat2_tx, beat2_rx) = chan::<W<32, 4>, DefaultClock>();
        let (rl1_tx, rl1_rx) = chan::<Grant<IW>, DefaultClock>();
        let (rl2_tx, rl2_rx) = chan::<Grant<IW>, DefaultClock>();
        // The wires the watch reads.
        let (halt1_o, halt1_i) = signal::<Bit, DefaultClock>();
        let (halt2_o, halt2_i) = signal::<Bit, DefaultClock>();
        let (instr1_o, instr1_i) = signal::<U<32>, DefaultClock>();
        let (instr2_o, instr2_i) = signal::<U<32>, DefaultClock>();
        let (wb1_o, wb1_i) = signal::<Writeback, DefaultClock>();
        let (wb2_o, wb2_i) = signal::<Writeback, DefaultClock>();
        let (d1_o, d1_i) = signal::<Bit, DefaultClock>();
        let (d2_o, d2_i) = signal::<Bit, DefaultClock>();
        let (d3_o, d3_i) = signal::<Bit, DefaultClock>();
        let (rst2_o, rst2_i) = signal::<Bit, DefaultClock>();
        join2(
            join2(
                join2(
                    self.inject.run((rst_inj, fault), rst2_o),
                    self.tee_rdata.run(rdata, (rd1_tx, rd2_tx)),
                ),
                join2(
                    self.tee_done.run(done, (dn1_tx, dn2_tx)),
                    self.tee_grant.run(grant, (gr1_tx, gr2_tx)),
                ),
            ),
            join2(
                join2(
                    self.one.run(
                        (rst_one, irq_one, tirq, rd1_rx, dn1_rx, gr1_rx),
                        (halt1_o, instr1_o, wb1_o, is1_tx, beat1_tx, rl1_tx),
                    ),
                    self.two.run(
                        (rst2_i, irq2, tirq_two, rd2_rx, dn2_rx, gr2_rx),
                        (halt2_o, instr2_o, wb2_o, is2_tx, beat2_tx, rl2_tx),
                    ),
                ),
                join2(
                    join2(
                        self.check_issue
                            .run((rst_i, is1_rx, is2_rx), (issue, d1_o)),
                        self.check_wbeat
                            .run((rst_w, beat1_rx, beat2_rx), (wbeat, d2_o)),
                    ),
                    join2(
                        self.check_release
                            .run((rst_r, rl1_rx, rl2_rx), (release, d3_o)),
                        self.watch.run(
                            (
                                rst, halt1_i, halt2_i, instr1_i, instr2_i,
                                wb1_i, wb2_i, d1_i, d2_i, d3_i,
                            ),
                            (halt, instr, wb, differs),
                        ),
                    ),
                ),
            ),
        )
        .await;
    }
}
// end{run}
