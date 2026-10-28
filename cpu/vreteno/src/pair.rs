// SPDX-License-Identifier: Apache-2.0
//! Two cores running one program, and a line that says they disagreed.
//!
//! A machine that must not be quietly wrong runs two of itself and
//! compares. What makes it worth something is that both copies see
//! exactly the same inputs in the same cycles, so that any difference
//! in what they do is theirs and not their inputs'; and that the
//! comparison covers everything a core says, not a sample of it.
//!
//! `Pair` holds two `Vreteno` harts. Every input the pair is given
//! goes to both: the wires directly, and the three channels through a
//! `Tee` each, which offers each word to both cores and takes the
//! next only once both have taken it. Every output is compared: the
//! three channels through a `Check` each, which takes a word from
//! both cores in one cycle, passes the first core's on and raises a
//! line when the two differ,
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

use crate::core::Writeback;
use crate::hart::{Hart, HartIn, HartOut};

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

/// What `Watch` compares (issue 726): the reset, each core's halt,
/// instruction and writeback, and the three checks' lines. Taken apart
/// in `run`, so each port keeps its name.
pub struct WatchIn {
    pub rst: In<Bit>,
    pub halt1: In<Bit>,
    pub halt2: In<Bit>,
    pub instr1: In<U<32>>,
    pub instr2: In<U<32>>,
    pub wb1: In<Writeback>,
    pub wb2: In<Writeback>,
    pub d1: In<Bit>,
    pub d2: In<Bit>,
    pub d3: In<Bit>,
}

/// What `Watch` passes on (issue 726): the first core's halt,
/// instruction and writeback, and the line raised on a disagreement.
pub struct WatchOut {
    pub halt: Out<Bit>,
    pub instr: Out<U<32>>,
    pub wb: Out<Writeback>,
    pub differs: Out<Bit>,
}

#[lower]
impl Unit for Watch {
    async fn run(
        &mut self,
        WatchIn {
            rst,
            halt1,
            halt2,
            instr1,
            instr2,
            wb1,
            wb2,
            d1,
            d2,
            d3,
        }: WatchIn,
        WatchOut {
            halt,
            instr,
            wb,
            differs,
        }: WatchOut,
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
    pub one: Hart<IW>,
    /// The core that only checks.
    pub two: Hart<IW>,
    /// The read answers, to both cores.
    pub tee_rdata: Tee<R![DATA = 32, ID = IW]>,
    /// The write completions, to both.
    pub tee_done: Tee<Done<IW>>,
    /// The identifiers handed out, to both.
    pub tee_grant: Tee<Grant<IW>>,
    /// The bursts they issue, compared.
    pub check_issue: Check<Issue<32>>,
    /// The write beats they send, compared.
    pub check_wbeat: Check<W![DATA = 32, STRB = 4]>,
    /// The identifiers they release, compared.
    pub check_release: Check<Grant<IW>>,
    /// Their three wires, compared, and passed on.
    pub watch: Watch,
    /// The self-test's one difference.
    pub inject: Inject,
}
// end{state}

/// What the pair takes in (issue 726): the reset, the interrupt lines
/// and the time, the data cache's snoop, the fault to inject, and the
/// bus's answers. Taken apart in `run`, so each port keeps its name.
pub struct PairIn<const IW: usize> {
    pub rst: In<Bit>,
    pub irq: In<Bit>,
    pub tirq: In<Bit>,
    pub sirq: In<Bit>,
    pub time: In<U<64>>,
    pub seirq: In<Bit>,
    pub dc_snoop: In<U<9>>,
    pub fault: In<Bit>,
    pub rdata: Rx<R<32, IW>>,
    pub done: Rx<Done<IW>>,
    pub grant: Rx<Grant<IW>>,
}

/// What the pair gives out (issue 726): the first core's halt,
/// instruction and writeback, its requests on the bus, and the line
/// raised when the two cores disagree.
pub struct PairOut<const IW: usize> {
    pub halt: Out<Bit>,
    pub instr: Out<U<32>>,
    pub wb: Out<Writeback>,
    pub issue: Tx<Issue<32>>,
    pub wbeat: Tx<W<32, 4>>,
    pub release: Tx<Grant<IW>>,
    pub differs: Out<Bit>,
}

// begin{run}
#[lower]
impl<const IW: usize> Unit for Pair<IW> {
    async fn run(
        &mut self,
        PairIn {
            rst,
            irq,
            tirq,
            sirq,
            time,
            seirq,
            dc_snoop,
            fault,
            rdata,
            done,
            grant,
        }: PairIn<IW>,
        PairOut {
            halt,
            instr,
            wb,
            issue,
            wbeat,
            release,
            differs,
        }: PairOut<IW>,
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
        let sirq_two = sirq.clone();
        let time_two = time.clone();
        let seirq_two = seirq.clone();
        let dc_snoop_two = dc_snoop.clone();
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
        // No debugger on either core: the requests stay low, and what
        // each says about debug mode goes nowhere.
        let (_hr1_o, hr1_i) = signal::<Bit, DefaultClock>();
        let (_rr1_o, rr1_i) = signal::<Bit, DefaultClock>();
        let (dbg1_o, _dbg1_i) = signal::<Bit, DefaultClock>();
        let (_hr2_o, hr2_i) = signal::<Bit, DefaultClock>();
        let (_rr2_o, rr2_i) = signal::<Bit, DefaultClock>();
        let (dbg2_o, _dbg2_i) = signal::<Bit, DefaultClock>();
        // Nor a debug module: no register access on either.
        let (_rn1_o, rn1_i) = signal::<U<16>, DefaultClock>();
        let (_wd1_o, wd1_i) = signal::<U<32>, DefaultClock>();
        let (_we1_o, we1_i) = signal::<Bit, DefaultClock>();
        let (rd1_o, _rd1_i) = signal::<U<32>, DefaultClock>();
        // Whether each waits in `wfi`, which the pair does not compare
        // (issue 1408).
        let (sl1_o, _sl1_i) = signal::<Bit, DefaultClock>();
        let (sl2_o, _sl2_i) = signal::<Bit, DefaultClock>();
        let (_rn2_o, rn2_i) = signal::<U<16>, DefaultClock>();
        let (_wd2_o, wd2_i) = signal::<U<32>, DefaultClock>();
        let (_we2_o, we2_i) = signal::<Bit, DefaultClock>();
        let (rd2_o, _rd2_i) = signal::<U<32>, DefaultClock>();
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
                        HartIn {
                            rst: rst_one,
                            irq: irq_one,
                            tirq,
                            sirq,
                            rdata: rd1_rx,
                            done: dn1_rx,
                            grant: gr1_rx,
                            haltreq: hr1_i,
                            resumereq: rr1_i,
                            dbg_regno: rn1_i,
                            dbg_wdata: wd1_i,
                            dbg_we: we1_i,
                            time,
                            seirq,
                            dc_snoop,
                        },
                        HartOut {
                            halt: halt1_o,
                            instr: instr1_o,
                            wb: wb1_o,
                            issue: is1_tx,
                            wbeat: beat1_tx,
                            release: rl1_tx,
                            dbg: dbg1_o,
                            dbg_rdata: rd1_o,
                            asleep: sl1_o,
                        },
                    ),
                    self.two.run(
                        HartIn {
                            rst: rst2_i,
                            irq: irq2,
                            tirq: tirq_two,
                            sirq: sirq_two,
                            rdata: rd2_rx,
                            done: dn2_rx,
                            grant: gr2_rx,
                            haltreq: hr2_i,
                            resumereq: rr2_i,
                            dbg_regno: rn2_i,
                            dbg_wdata: wd2_i,
                            dbg_we: we2_i,
                            time: time_two,
                            seirq: seirq_two,
                            dc_snoop: dc_snoop_two,
                        },
                        HartOut {
                            halt: halt2_o,
                            instr: instr2_o,
                            wb: wb2_o,
                            issue: is2_tx,
                            wbeat: beat2_tx,
                            release: rl2_tx,
                            dbg: dbg2_o,
                            dbg_rdata: rd2_o,
                            asleep: sl2_o,
                        },
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
                            WatchIn {
                                rst,
                                halt1: halt1_i,
                                halt2: halt2_i,
                                instr1: instr1_i,
                                instr2: instr2_i,
                                wb1: wb1_i,
                                wb2: wb2_i,
                                d1: d1_i,
                                d2: d2_i,
                                d3: d3_i,
                            },
                            WatchOut {
                                halt,
                                instr,
                                wb,
                                differs,
                            },
                        ),
                    ),
                ),
            ),
        )
        .await;
    }
}
// end{run}
