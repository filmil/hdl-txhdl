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
//!
//! `raw` is for the measurement and nothing else: a program that
//! reads it in a loop and sends the bits up the serial line is how the
//! source's bias and its entropy are estimated on the board.
use txhdl::comp::trace::Kind;
use txhdl::comp::{
    join2, mux, signal, Clock, DefaultClock, In, Mem, Out, Reg, Unit,
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
// The peripheral's four words.
regmap! { regs (regs_read, regs_we, regs_re), 2: [
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
    /// The model's registers, one per ring.
    pub lfsr: Mem<U<32>, RINGS>,
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
                    self.lfsr.write(i, U::<32>::from(*seed));
                }
                self.seeded.set(Bit::One);
                raw.set(U::<RINGS>::from(0u8));
                continue;
            }
            let mut out = 0u32;
            for i in 0..RINGS {
                let s = self.lfsr.read(i).raw() as u32;
                let fb = (s >> 31) ^ (s >> 21) ^ (s >> 1) ^ s;
                let next = (s << 1) | (fb & 1);
                if en.get().to_bool() {
                    self.lfsr.write(i, U::<32>::from(next));
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
            // The bus.
            let arh = bus.ar.head();
            let awh = bus.aw.head();
            let wh = bus.w.head();
            let rsel = arh.addr.slice::<2, 2>();
            let wsel = awh.addr.slice::<2, 2>();
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
            let status = regs_status_pack(ready, count, fault, running);
            let data = mux(ready, self.words.read(head), U::<32>::from(0u8));
            let ctrl = regs_ctrl_pack(running, Bit::Zero);
            let answer = regs_read(rsel, data, status, ctrl, rawv);
            with!(self <= {
                to_ctrl ? run: regs_ctrl_run(written),
                to_ctrl & regs_ctrl_clear(written) ? fault: Bit::Zero,
                // A trip in the cycle of a clear stays tripped.
                tripped ? fault: Bit::One,
                running ? {
                    prev: bit,
                    runlen: run1,
                    rawv: (rawv << 1u32) | bit.zext::<32>(),
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
        for _ in 0..200_000 {
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
