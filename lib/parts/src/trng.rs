// SPDX-License-Identifier: Apache-2.0
//! An entropy source: ring oscillators sampled, folded, debiased,
//! checked and buffered behind AXI-Lite.
//!
//! A network stack wants numbers nobody can predict, and a machine
//! that has no clock of the day and boots into the same state every
//! time has nowhere to get them but physics (issue 458). The physics
//! here is the jitter of ring oscillators: a loop of an odd number of
//! inverters runs at a rate set by its own gates, wandering with the
//! temperature and the supply, and where the clock's edge catches it
//! in its period is not something a program can know. That half is
//! [`RingOsc`], a Verilog module the netlist instantiates and does
//! not write, since a loop of gates is not a thing this language
//! says; in a Rust run it is a model, a shift register with feedback,
//! which is stated here so that nobody reads a simulation as proof of
//! randomness. Only a board proves that, and issue 458 says how far
//! this has been measured.
//!
//! The other half, [`Trng`], is ordinary hardware and is lowered. Each
//! cycle it folds the rings' samples to one bit by exclusive or, which
//! makes a bit less biased than any one ring's. It then does three
//! things with the stream:
//!
//! * **Watches it.** A ring that has stopped, or been made to stop,
//!   gives the same bit for ever, and a source that keeps handing out
//!   words then is worse than none. The repetition count test of NIST
//!   SP 800-90B is a counter: [`RCT_CUTOFF`] identical bits in a row
//!   raise a sticky fault, the buffer stops filling, and a host reads
//!   why in `status`. The cutoff is `1 + 20 / H` for a false alarm rate
//!   of one in a million at a min-entropy `H` of half a bit per
//!   sample, rounded to forty.
//! * **Debiases it.** Von Neumann's extractor takes the stream in
//!   pairs: `01` gives a zero, `10` a one, and `00` and `11` give
//!   nothing. A bias in the source cancels, since the two orders of a
//!   pair are equally likely whatever the bias is, at the cost of
//!   three bits in four on average.
//! * **Buffers it.** The bits that survive are shifted into a word,
//!   and whole words go into a buffer of four, which a host reads a
//!   word at a time. A word that finds the buffer full is dropped; the
//!   source makes more.
//!
//! | Offset | Name | What it is |
//! |---|---|---|
//! | `0x0` | `data` | a word of entropy, taken by the read; zero if none |
//! | `0x4` | `status` | bit 0 a word is ready, bits 3 to 1 how many, bit 8 the fault, bit 9 running |
//! | `0x8` | `ctrl` | bit 0 run; a write with bit 1 clears the fault |
//! | `0xc` | `raw` | the last 32 folded samples, before the extractor |
//! | `0x10` | `cap` | a write of bit 0 starts a capture; bit 1 busy, bit 2 done |
//! | `0x14` | `capidx` | which captured word `capword` reads, 0 to 255 |
//! | `0x18` | `capword` | that captured word, 32 samples, the newest in bit 0 |
//!
//! `raw` is for the measurement and nothing else: a program that
//! reads it in a loop and sends the bits up the serial line is how the
//! source's bias and its entropy are estimated on the board.
//!
//! A read of `raw` sees 32 samples, and a loop on the board cannot
//! read it again within 32 cycles, so the samples of two reads never
//! meet and nothing longer than a window can be measured that way.
//! The capture is the long measurement: a write of `cap` fills a
//! buffer of `CAP_WORDS` words from the samples as they come, one word
//! every 32 cycles while the source runs, so the words are 8192
//! samples in a row with none missing, and a program reads them out
//! afterwards at any pace (#817). It is a measurement aid; the
//! extractor and the health test run on regardless.
use txhdl::comp::trace::Kind;
use txhdl::comp::{
    join2, mux, signal, Clock, DefaultClock, In, Mem, Out, Reg, Regs, Unit,
};
use txhdl::netlist::{foreign, Lower, Lowered};
use txhdl::types::{Bit, U};
use txhdl::{lower, regmap, with, Trace};

use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteB, LitePort, LiteR};

/// How many rings the module holds and the peripheral folds.
pub const RINGS: usize = 8;
/// Inverters in a ring. Odd, or the loop would settle.
pub const RING_LENGTH: i128 = 7;
/// Identical folded samples in a row that raise the fault.
pub const RCT_CUTOFF: u32 = 40;
/// How many words the buffer holds.
pub const WORDS: usize = 4;
/// How many words a capture holds: 8192 samples in a row.
pub const CAP_WORDS: usize = 256;
/// The model rings' seeds, one a ring, apart from each other so that
/// the eight streams do not start alike; the same eight are in the
/// module's simulation branch.
pub const SEEDS: [u32; RINGS] = [
    0x9e37_79b9,
    0x7f4a_7c15,
    0x2545_f491,
    0x6c8e_9cf5,
    0x1b87_3593,
    0xcc9e_2d51,
    0x85eb_ca6b,
    0xc2b2_ae35,
];

// begin{regs}
// The peripheral's seven words.
regmap! { regs (regs_read, regs_we, regs_re), 3: [
    (0, data, rc, "the oldest word of entropy"),
    (1, status, ro, "the buffer and the health test", [
        (ready, 0, 1, ro, 0, "a word is ready"),
        (count, 1, 3, ro, 0, "how many words wait"),
        (fault, 8, 1, ro, 0, "the repetition count test tripped"),
        (run, 9, 1, ro, 0, "the run bit, read back"),
    ]),
    (2, ctrl, rw, "the run bit, and the fault's clear", [
        (run, 0, 1, rw, 0, "the rings run and the buffer fills"),
        (clear, 1, 1, wo, 0, "written one, the fault is cleared"),
    ]),
    (3, raw, ro, "the last 32 folded samples"),
    (4, cap, rw, "a capture of consecutive samples", [
        (start, 0, 1, wo, 0, "written one, a capture begins"),
        (busy, 1, 1, ro, 0, "the capture is filling"),
        (done, 2, 1, ro, 0, "the capture is whole"),
    ]),
    (5, capidx, rw, "which captured word capword reads", [
        (index, 0, 8, rw, 0, "the word, 0 to 255"),
    ]),
    (6, capword, ro, "the captured word capidx names, newest in bit 0"),
] }
// end{regs}

/// The word of entropy, as an offset from the peripheral's base.
pub const DATA: u32 = regs::data;
/// The status word.
pub const STATUS: u32 = regs::status;
/// The control word.
pub const CTRL: u32 = regs::ctrl;
/// The last 32 folded samples.
pub const RAW: u32 = regs::raw;
/// The capture's control and status.
pub const CAP: u32 = regs::cap;
/// Which captured word `CAPWORD` reads.
pub const CAPIDX: u32 = regs::capidx;
/// The captured word `CAPIDX` names.
pub const CAPWORD: u32 = regs::capword;
/// `cap` bit 0, on a write: a capture begins.
pub const CAP_START: u32 = regs::cap_start.mask();
/// `cap` bit 1: the capture is filling.
pub const CAP_BUSY: u32 = regs::cap_busy.mask();
/// `cap` bit 2: the capture is whole.
pub const CAP_DONE: u32 = regs::cap_done.mask();

/// `ctrl` bit 0: the rings run and the buffer fills.
pub const CTRL_RUN: u32 = regs::ctrl_run.mask();
/// `ctrl` bit 1, on a write: the fault is cleared.
pub const CTRL_CLEAR: u32 = regs::ctrl_clear.mask();
/// `status` bit 0: a word is ready.
pub const STATUS_READY: u32 = regs::status_ready.mask();
/// `status` bit 8: the repetition count test tripped.
pub const STATUS_FAULT: u32 = regs::status_fault.mask();
/// `status` bit 9: the run bit, read back.
pub const STATUS_RUN: u32 = regs::status_run.mask();

// begin{ring}
/// The rings: `RINGS` ring oscillators, sampled on the clock, as the
/// foreign module `ring_osc` in `lib/parts/hdl/ring_osc.v`.
///
/// In a Rust run this is a model and says so: a shift register with
/// feedback per ring, seeded apart, which has the shape of the samples
/// and none of their physics. `en` low holds every ring still and the
/// samples at zero, as the module does.
#[derive(Trace, Default)]
pub struct RingOsc {
    /// The model's registers, one per ring. Registers and not a
    /// memory: all eight are seeded in one cycle and all eight step in
    /// every cycle after, and a memory takes one write a cycle, so as a
    /// memory the model was ring 7 alone (issue 809).
    pub lfsr: Regs<U<32>, RINGS>,
    /// Whether the model has been seeded.
    pub seeded: Reg<Bit>,
}

impl Lower for RingOsc {
    fn lowered_as(name: &str) -> Lowered {
        foreign(
            name,
            "ring_osc",
            &[("en", Kind::In, 1), ("raw", Kind::Out, RINGS)],
            &[("N", RINGS as i128), ("L", RING_LENGTH)],
            &[("clk", DefaultClock::NAME)],
        )
    }
}

impl Unit<In<Bit>, Out<U<RINGS>>> for RingOsc {
    async fn run(&mut self, en: In<Bit>, raw: Out<U<RINGS>>) {
        loop {
            DefaultClock::rising().await;
            if !self.seeded.get().to_bool() {
                for (i, seed) in SEEDS.iter().enumerate() {
                    self.lfsr[i].set(U::<32>::from(*seed));
                }
                self.seeded.set(Bit::One);
                raw.set(U::<RINGS>::from(0u8));
                continue;
            }
            let mut out = 0u32;
            for i in 0..RINGS {
                let s = self.lfsr[i].get().raw() as u32;
                let fb = (s >> 31) ^ (s >> 21) ^ (s >> 1) ^ s;
                let next = (s << 1) | (fb & 1);
                if en.get().to_bool() {
                    self.lfsr[i].set(U::<32>::from(next));
                    out |= (s >> 31) << i;
                }
            }
            raw.set(U::<RINGS>::from(out));
        }
    }
}
// end{ring}

// begin{state}
/// The conditioning and the registers, on AXI-Lite.
#[derive(Trace, Default)]
pub struct Trng {
    /// Run: the rings on and the buffer filling.
    pub run: Reg<Bit>,
    /// The repetition count test tripped. Sticky until cleared.
    pub fault: Reg<Bit>,
    /// The last folded sample, for the test.
    pub prev: Reg<Bit>,
    /// How many identical samples in a row, counting the last.
    pub runlen: Reg<U<6>>,
    /// The last 32 folded samples.
    pub rawv: Reg<U<32>>,
    /// Whether the first bit of a pair is held.
    pub have: Reg<Bit>,
    /// The first bit of the pair.
    pub first: Reg<Bit>,
    /// The bits that survived, shifted in from the top.
    pub shift: Reg<U<32>>,
    /// How many of them.
    pub nbits: Reg<U<6>>,
    /// The words waiting to be read.
    pub words: Mem<U<32>, WORDS>,
    /// The oldest word waiting.
    pub head: Reg<U<2>>,
    /// Where the next word goes.
    pub tail: Reg<U<2>>,
    /// How many words are waiting, up to `WORDS`.
    pub count: Reg<U<3>>,
    /// The capture: `CAP_WORDS` words of samples in a row (#817).
    pub cap: Mem<U<32>, CAP_WORDS>,
    /// A capture is filling.
    pub cbusy: Reg<Bit>,
    /// A capture is whole.
    pub cdone: Reg<Bit>,
    /// The capture's word being filled.
    pub cword: Reg<U<8>>,
    /// Samples into that word, less one: at 31 the word is written.
    pub cbits: Reg<U<5>>,
    /// Which captured word `capword` reads.
    pub cidx: Reg<U<8>>,
}
// end{state}

// begin{run}
#[lower]
impl Unit for Trng {
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        (raw, en): (In<U<RINGS>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let running = self.run.get();
            let fault = self.fault.get();
            let prev = self.prev.get();
            let runlen = self.runlen.get();
            let rawv = self.rawv.get();
            let have = self.have.get();
            let first = self.first.get();
            let shift = self.shift.get();
            let nbits = self.nbits.get();
            let head = self.head.get();
            let tail = self.tail.get();
            let count = self.count.get();
            let cbusy = self.cbusy.get();
            let cdone = self.cdone.get();
            let cword = self.cword.get();
            let cbits = self.cbits.get();
            let cidx = self.cidx.get();
            // The bus.
            let arh = bus.ar.head();
            let awh = bus.aw.head();
            let wh = bus.w.head();
            let rsel = arh.addr.slice::<2, 3>();
            let wsel = awh.addr.slice::<2, 3>();
            let rgo = bus.r.ready() & bus.ar.peek().is_some();
            let _ = bus.ar.recv_if(bus.r.ready());
            let wgo = bus.b.ready()
                & bus.aw.peek().is_some()
                & bus.w.peek().is_some();
            let _ = bus.aw.recv_if(wgo);
            let _ = bus.w.recv_if(wgo);
            let written = wh.data;
            // The sample: the rings' bits folded to one.
            let r = raw.get();
            let bit = r.bit(0)
                ^ r.bit(1)
                ^ r.bit(2)
                ^ r.bit(3)
                ^ r.bit(4)
                ^ r.bit(5)
                ^ r.bit(6)
                ^ r.bit(7);
            // The repetition count test: the run of identical samples,
            // and whether this one makes it the cutoff.
            let same = Bit::from(bit == prev);
            let run1 = mux(same, runlen + 1, U::<6>::from(1u8));
            let tripped = running & same & (runlen >= RCT_CUTOFF - 1);
            // Von Neumann: the second bit of a pair, and whether the
            // pair says anything.
            let taking = running & !fault;
            let pair = taking & have;
            let keep = pair & Bit::from(first != bit);
            let next = (shift >> 1u32) | (first.zext::<32>() << 31u32);
            let word_done = keep & (nbits == 31);
            let full = count == WORDS as u32;
            let push = word_done & !full;
            let ready = Bit::from(count != 0);
            // A read of the data word takes the oldest word, if one
            // waits.
            let pop = regs_re(rgo, rsel).bit(0) & ready;
            // A write to the control word.
            let to_ctrl = regs_we(wgo, wsel).bit(2);
            // The capture (#817): a write of `cap` with bit 0 starts
            // it, and while the source runs every sample goes into the
            // word being filled, which is written whole when it holds
            // 32, the same 32 `raw` would then read, so the words are
            // samples in a row with none missing and none twice.
            let start = regs_we(wgo, wsel).bit(4) & regs_cap_start(written);
            let to_idx = regs_we(wgo, wsel).bit(5);
            let sampled = (rawv << 1u32) | bit.zext::<32>();
            let cstep = cbusy & running;
            let cword_done = cstep & (cbits == 31);
            let clast = cword_done & (cword == CAP_WORDS as u32 - 1);
            let cap_status = regs_cap_pack(Bit::Zero, cbusy, cdone);
            let capidx_word = regs_capidx_pack(cidx);
            let capword = self.cap.read(cidx);
            let status = regs_status_pack(ready, count, fault, running);
            let data = mux(ready, self.words.read(head), U::<32>::from(0u8));
            let ctrl = regs_ctrl_pack(running, Bit::Zero);
            let answer = regs_read(
                rsel,
                data,
                status,
                ctrl,
                rawv,
                cap_status,
                capidx_word,
                capword,
            );
            with!(self <= {
                to_ctrl ? run: regs_ctrl_run(written),
                to_ctrl & regs_ctrl_clear(written) ? fault: Bit::Zero,
                // A trip in the cycle of a clear stays tripped.
                tripped ? fault: Bit::One,
                running ? {
                    prev: bit,
                    runlen: run1,
                    rawv: sampled,
                },
                taking ? have: !have,
                taking & !have ? first: bit,
                keep & !word_done ? {
                    shift: next,
                    nbits: nbits + 1,
                },
                word_done ? {
                    shift: U::<32>::from(0u8),
                    nbits: U::<6>::from(0u8),
                },
                push ? {
                    words.at(tail): next,
                    tail: tail + 1,
                },
                pop ? head: head + 1,
                push & !pop ? count: count + 1,
                pop & !push ? count: count - 1,
                cstep ? cbits: cbits + 1,
                cword_done ? {
                    cap.at(cword): sampled,
                    cword: cword + 1,
                },
                clast ? {
                    cbusy: Bit::Zero,
                    cdone: Bit::One,
                },
                start ? {
                    cbusy: Bit::One,
                    cdone: Bit::Zero,
                    cword: U::<8>::from(0u8),
                    cbits: U::<5>::from(0u8),
                },
                to_idx ? cidx: regs_capidx_index(written),
            });
            en.set(running);
            if rgo.to_bool() {
                bus.r.send(LiteR {
                    data: answer,
                    resp: Resp::Okay,
                });
            }
            if wgo.to_bool() {
                bus.b.send(LiteB { resp: Resp::Okay });
            }
        }
    }
}
// end{run}

// begin{entropy}
/// The source whole: the rings and the peripheral, joined by the
/// samples one way and the enable the other, so that a design holds
/// one unit and the netlist has the module inside it.
#[derive(Trace, Default)]
pub struct Entropy {
    /// The rings.
    pub ring: RingOsc,
    /// The peripheral.
    pub trng: Trng,
}

#[lower]
impl Unit for Entropy {
    async fn run(&mut self, bus: LitePort<32, 32, 4>, _o: ()) {
        let (raw_o, raw_i) = signal::<U<RINGS>, DefaultClock>();
        let (en_o, en_i) = signal::<Bit, DefaultClock>();
        join2(
            self.ring.run(en_i, raw_o),
            self.trng.run(
                LitePort {
                    aw: bus.aw,
                    ar: bus.ar,
                    w: bus.w,
                    b: bus.b,
                    r: bus.r,
                },
                (raw_i, en_o),
            ),
        )
        .await;
    }
}
// end{entropy}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::axi_lite::{axi_lite, LiteAw, LiteHost, LiteW};
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::Running;

    type Host = LiteHost<32, 32, 4>;

    async fn write(h: &Host, addr: u32, data: u32) {
        let (aw, _, w, b, _) = h;
        aw.send(LiteAw {
            addr: U::from(addr),
            prot: U::from(0u8),
        });
        w.send(LiteW {
            data: U::from(data),
            strb: U::from(0xfu8),
        });
        loop {
            DefaultClock::rising().await;
            if b.recv().is_some() {
                return;
            }
        }
    }

    async fn read(h: &Host, addr: u32) -> u32 {
        let (_, ar, _, _, r) = h;
        ar.send(LiteAw {
            addr: U::from(addr),
            prot: U::from(0u8),
        });
        loop {
            DefaultClock::rising().await;
            if let Some(got) = r.recv() {
                return got.data.raw() as u32;
            }
        }
    }

    /// A word of entropy, waited for.
    async fn word(h: &Host) -> u32 {
        loop {
            if read(h, STATUS).await & STATUS_READY != 0 {
                return read(h, DATA).await;
            }
        }
    }

    /// Run a client against the peripheral fed by `feed`, a source of
    /// samples per cycle; `None` means the model rings.
    fn run<F>(
        feed: Option<Rc<dyn Fn(u64) -> u32>>,
        client: impl FnOnce(Host) -> F,
    ) where
        F: std::future::Future<Output = ()>,
    {
        run_for(feed, 200_000, client);
    }

    /// The same, for up to `cycles` cycles.
    fn run_for<F>(
        feed: Option<Rc<dyn Fn(u64) -> u32>>,
        cycles: u64,
        client: impl FnOnce(Host) -> F,
    ) where
        F: std::future::Future<Output = ()>,
    {
        let link = axi_lite::<32, 32, 4>();
        let bus: LitePort<32, 32, 4> = link.per.into();
        let done = Rc::new(RefCell::new(false));
        let d = done.clone();
        let body = client(link.host);
        let client = async move {
            body.await;
            *d.borrow_mut() = true;
        };
        let mut trng = Trng::default();
        let mut ring = RingOsc::default();
        let (raw_o, raw_i) = signal::<U<RINGS>, DefaultClock>();
        let (en_o, en_i) = signal::<Bit, DefaultClock>();
        let source = async move {
            match feed {
                None => ring.run(en_i, raw_o).await,
                Some(f) => {
                    let mut t = 0u64;
                    loop {
                        DefaultClock::rising().await;
                        raw_o.set(U::<RINGS>::from(f(t)));
                        t += 1;
                    }
                }
            }
        };
        let mut sim = Running::new(join2(
            join2(client, source),
            trng.run(bus, (raw_i, en_o)),
        ));
        for _ in 0..cycles {
            sim.cycle();
            if *done.borrow() {
                return;
            }
        }
        panic!("the client did not finish");
    }

    #[test]
    fn nothing_comes_until_it_runs() {
        run(None, |h| async move {
            for _ in 0..300 {
                DefaultClock::rising().await;
            }
            assert_eq!(read(&h, STATUS).await, 0, "stopped, empty, no fault");
            assert_eq!(
                read(&h, DATA).await,
                0,
                "and a read of nothing is zero"
            );
        });
    }

    #[test]
    fn words_come_and_differ() {
        run(None, |h| async move {
            write(&h, CTRL, CTRL_RUN).await;
            let mut seen = Vec::new();
            for _ in 0..8 {
                seen.push(word(&h).await);
            }
            let mut sorted = seen.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(
                sorted.len(),
                8,
                "eight words, all different: {seen:x?}"
            );
            assert_eq!(
                read(&h, STATUS).await & STATUS_FAULT,
                0,
                "the model rings do not trip the test"
            );
        });
    }

    /// The buffer holds four words and no more, and reads take them
    /// oldest first.
    #[test]
    fn the_buffer_holds_four() {
        run(None, |h| async move {
            write(&h, CTRL, CTRL_RUN).await;
            for _ in 0..2000 {
                DefaultClock::rising().await;
            }
            let s = read(&h, STATUS).await;
            assert_eq!((s >> 1) & 7, 4, "four words waiting: {s:#x}");
            // Stopped, so the words read out are not replaced.
            write(&h, CTRL, 0).await;
            for i in (1..=4).rev() {
                let s = read(&h, STATUS).await;
                assert_eq!((s >> 1) & 7, i);
                read(&h, DATA).await;
            }
            assert_eq!(read(&h, STATUS).await & STATUS_READY, 0, "drained");
        });
    }

    /// The extractor: a source that alternates gives all zeros, since
    /// every pair is `01`, and a stuck source gives nothing at all.
    #[test]
    fn von_neumann_takes_the_order_of_a_pair() {
        run(Some(Rc::new(|t| (t & 1) as u32)), |h| async move {
            write(&h, CTRL, CTRL_RUN).await;
            let w = word(&h).await;
            assert_eq!(w, 0, "every pair is 01, so every bit is 0: {w:#x}");
        });
        run(Some(Rc::new(|t| ((t + 1) & 1) as u32)), |h| async move {
            write(&h, CTRL, CTRL_RUN).await;
            let w = word(&h).await;
            assert_eq!(w, 0xffff_ffff, "every pair is 10: {w:#x}");
        });
    }

    /// A stuck source trips the repetition count test after the
    /// cutoff, the buffer stops, and a clear starts it again.
    #[test]
    fn a_stuck_source_is_a_fault() {
        run(Some(Rc::new(|_| 0xffu32)), |h| async move {
            write(&h, CTRL, CTRL_RUN).await;
            for _ in 0..RCT_CUTOFF as usize + 4 {
                DefaultClock::rising().await;
            }
            let s = read(&h, STATUS).await;
            assert_ne!(s & STATUS_FAULT, 0, "tripped: {s:#x}");
            assert_eq!(s & STATUS_READY, 0, "and nothing was handed out");
            write(&h, CTRL, CTRL_RUN | CTRL_CLEAR).await;
            // Still stuck, so it trips again, but it was clear for a
            // moment: the clear takes.
            let s = read(&h, STATUS).await;
            assert_ne!(s & STATUS_RUN, 0, "still running");
        });
        // A source that is only mostly stuck stays under the cutoff.
        run(
            Some(Rc::new(|t| if t % 30 == 0 { 1 } else { 0 })),
            |h| async move {
                write(&h, CTRL, CTRL_RUN).await;
                for _ in 0..400 {
                    DefaultClock::rising().await;
                }
                let s = read(&h, STATUS).await;
                assert_eq!(s & STATUS_FAULT, 0, "under the cutoff: {s:#x}");
            },
        );
    }

    /// A sample of a source with a stated law: splitmix64 of the cycle,
    /// so each cycle's draw is independent of every other's and the run
    /// is the same every time.
    fn draw(t: u64) -> f64 {
        let mut z = t.wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }

    /// `n` words of the extractor's output from the source `feed`.
    fn words(feed: Rc<dyn Fn(u64) -> u32>, n: usize) -> Vec<u32> {
        let got = Rc::new(RefCell::new(Vec::new()));
        let g = got.clone();
        run_for(Some(feed), 400 * n as u64, |h| async move {
            write(&h, CTRL, CTRL_RUN).await;
            for _ in 0..n {
                let w = word(&h).await;
                g.borrow_mut().push(w);
            }
            let s = read(&h, STATUS).await;
            assert_eq!(s & STATUS_FAULT, 0, "the source did not trip: {s:#x}");
        });
        let w = got.borrow().clone();
        w
    }

    /// The correlation of a word's bits with the bits `lag` further on
    /// in the same word, over all the words. Pairs stay inside a word,
    /// since two words read one after the other were not made one
    /// after the other.
    fn lag_corr(words: &[u32], lag: usize) -> (f64, f64) {
        let (mut sx, mut sy, mut sxx, mut syy, mut sxy, mut n) =
            (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        for w in words {
            for i in 0..32 - lag {
                let x = (w >> i & 1) as f64;
                let y = (w >> (i + lag) & 1) as f64;
                sx += x;
                sy += y;
                sxx += x * x;
                syy += y * y;
                sxy += x * y;
                n += 1.0;
            }
        }
        let r = (n * sxy - sx * sy)
            / ((n * sxx - sx * sx) * (n * syy - sy * sy)).sqrt();
        (r, r * n.sqrt())
    }

    /// The extractor adds no correlation of its own (issue 780): fed a
    /// source whose samples are independent and biased, one in six-
    /// tenths, its words carry no correlation at any lag from one to
    /// eight beyond four standard deviations, and are balanced.
    ///
    /// On the board the words were correlated. This is what says the
    /// cause is not the pairing or the packing but the samples, which
    /// the next test shows the extractor passes through.
    #[test]
    fn an_independent_biased_source_gives_uncorrelated_words() {
        let ws = words(Rc::new(|t| u32::from(draw(t) < 0.6)), 1024);
        for lag in 1..=8 {
            let (r, z) = lag_corr(&ws, lag);
            assert!(z.abs() < 4.0, "lag {lag}: r {r:+.4}, z {z:+.1}");
        }
        let ones: u32 = ws.iter().map(|w| w.count_ones()).sum();
        let bits = 32.0 * ws.len() as f64;
        let z = (ones as f64 - bits / 2.0) / (bits.sqrt() / 2.0);
        assert!(z.abs() < 4.0, "balanced: z {z:+.1}");
    }

    /// And what von Neumann cannot do: a source whose samples are fair
    /// and uncorrelated with their neighbour, but correlated with the
    /// sample two cycles on, gives correlated words. A pair is two
    /// neighbours, so the extractor sees the second-neighbour
    /// correlation between one pair and the next. The board's raw
    /// samples have that shape (issue 780): no correlation at lag one,
    /// and a positive one at lag two.
    #[test]
    fn a_source_correlated_two_apart_gives_correlated_words() {
        // Each sample copies the one two cycles back with probability a
        // quarter, and is a fresh fair draw otherwise.
        let src = Rc::new(RefCell::new(Vec::<u32>::new()));
        let feed = Rc::new(move |t: u64| {
            let mut s = src.borrow_mut();
            let t = t as usize;
            while s.len() <= t {
                let i = s.len() as u64;
                let b = if i >= 2 && draw(2 * i) < 0.25 {
                    s[i as usize - 2]
                } else {
                    u32::from(draw(2 * i + 1) < 0.5)
                };
                s.push(b);
            }
            s[t]
        });
        let ws = words(feed, 1024);
        let (r, z) = lag_corr(&ws, 1);
        assert!(z > 4.0, "lag 1 carries it through: r {r:+.4}, z {z:+.1}");
    }

    /// Every ring of the model steps from its own seed, as the
    /// simulation branch of `ring_osc.v` steps all eight: the samples
    /// are eight streams and not ring 7's alone (issue 809).
    #[test]
    fn the_model_steps_all_eight_rings() {
        let mut ring = RingOsc::default();
        let (raw_o, raw_i) = signal::<U<RINGS>, DefaultClock>();
        let (en_o, en_i) = signal::<Bit, DefaultClock>();
        en_o.set(Bit::One);
        let mut sim = Running::new(ring.run(en_i, raw_o));
        // The first edge seeds the rings.
        sim.cycle();
        let mut lfsr = SEEDS;
        for k in 0..64 {
            sim.cycle();
            let want = (0..RINGS).fold(0u32, |w, i| w | ((lfsr[i] >> 31) << i));
            assert_eq!(
                raw_i.get().raw() as u32,
                want,
                "the samples, cycle {k}"
            );
            for s in lfsr.iter_mut() {
                let fb = (*s >> 31) ^ (*s >> 21) ^ (*s >> 1) ^ *s;
                *s = (*s << 1) | (fb & 1);
            }
        }
    }

    /// The netlist instantiates the rings and does not write them.
    #[test]
    fn the_netlist_holds_the_rings_as_a_module() {
        let net = Entropy::lowered("entropy");
        let v = net.verilog();
        assert!(v.contains("ring_osc "), "{v}");
        assert!(v.contains(".N(8)"), "{v}");
        assert!(!v.contains("module ring_osc"), "{v}");
        let h = net.vhdl();
        assert!(h.contains("component ring_osc"), "{h}");
        assert!(!h.contains("entity ring_osc"), "{h}");
    }
}
