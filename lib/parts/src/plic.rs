// SPDX-License-Identifier: Apache-2.0
//! The platform-level interrupt controller: `Plic<N, EDGE>`, one unit
//! for every count of sources, with `Plic1` to `Plic8` naming the
//! counts.
//!
//! It is the RISC-V PLIC with one target, an AXI-Lite peripheral at
//! the standard offsets from its base:
//!
//! | offset      | word                                             |
//! |-------------|--------------------------------------------------|
//! | `0x4 * i`   | source `i`'s priority, 0 to 7; 0 never interrupts |
//! | `0x1000`    | the pending bits, bit `i` for source `i`; read only |
//! | `0x2000`    | the enable bits, the same way                    |
//! | `0x20_0000` | the threshold: a priority must be above it       |
//! | `0x20_0004` | claim on a read, complete on a write             |
//!
//! Every other offset reads as zero and ignores a write. Sources are
//! numbered from 1; source 0 does not exist, and its bits read zero.
//!
//! A source's line goes through a gateway first. A level source asks
//! while its line is high, and an edge source, one whose bit is set in
//! the `EDGE` parameter, asks on a rising edge. A request goes forward
//! and becomes pending only when the source has no request in service,
//! from the claim that took the last one to the complete that ended
//! it; an edge that comes during the service is held, one deep, and
//! asks on the complete. A level source whose line is still high on
//! the complete asks again at once.
//!
//! A read of the claim word answers with the number of the pending,
//! enabled source of the highest priority above the threshold, the
//! lowest number winning a tie, and clears its pending bit; it answers
//! 0 when there is none. Writing that number back is the complete.
//! The interrupt line is a register, high a cycle after a claim would
//! answer with a source. A write ignores its strobe.
use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteB, LitePort, LiteR};
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Regs, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

// begin{state}
/// A platform-level interrupt controller of `N` sources and one
/// target, `N` from 1 to 31, the most one word of pending bits holds.
/// `EDGE` has a bit per source, set for a source that asks on a rising
/// edge rather than while its line is high. One unit for every count:
/// the sources are an array of ports and the priorities an array of
/// registers, and the lowering unrolls the loops over them for each
/// `N` (issue 500). The words are 32 bits whatever `N` is, bit `s` for
/// source `s`, and the bits above `N` stay zero.
#[derive(Trace, Default)]
pub struct Plic<const N: usize, const EDGE: usize> {
    /// The priority of each source, `prio[s - 1]` for source `s`.
    pub prio: Regs<U<3>, N>,
    /// The target's threshold: only a priority above it interrupts.
    pub threshold: Reg<U<3>>,
    /// A request not yet claimed, a bit per source.
    pub pending: Reg<U<32>>,
    /// The sources the target takes, a bit per source.
    pub enable: Reg<U<32>>,
    /// A request forwarded and not yet completed, a bit per source.
    pub active: Reg<U<32>>,
    /// A rising edge that came while its source was active, kept to
    /// ask again on the complete, a bit per edge source.
    pub held: Reg<U<32>>,
    /// The lines as they were a cycle ago, to see an edge.
    pub prev: Reg<U<32>>,
    /// The interrupt line, as a register.
    pub asserted: Reg<Bit>,
}
// end{state}

/// One source.
pub type Plic1<const EDGE: usize> = Plic<1, EDGE>;
/// Two sources.
pub type Plic2<const EDGE: usize> = Plic<2, EDGE>;
/// Three sources.
pub type Plic3<const EDGE: usize> = Plic<3, EDGE>;
/// Four sources.
pub type Plic4<const EDGE: usize> = Plic<4, EDGE>;
/// Five sources.
pub type Plic5<const EDGE: usize> = Plic<5, EDGE>;
/// Six sources.
pub type Plic6<const EDGE: usize> = Plic<6, EDGE>;
/// Seven sources.
pub type Plic7<const EDGE: usize> = Plic<7, EDGE>;
/// Eight sources.
pub type Plic8<const EDGE: usize> = Plic<8, EDGE>;

// begin{ports}
// The lowering reads a loop over an array as `srcs[i]`, so the index is
// what it is written with, and Clippy would rather it were an iterator.
#[allow(clippy::needless_range_loop)]
#[lower]
impl<const N: usize, const EDGE: usize> Unit for Plic<N, EDGE> {
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        (rst, srcs, irq): (In<Bit>, [In<Bit>; N], Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            // end{ports}
            // begin{gateway}
            // The lines as one word, bit `s` for source `s`, bit 0 low.
            let rst = rst.get();
            let none = U::<32>::from(0u8);
            let one = U::<32>::from(1u8);
            let mut lines = none;
            for i in 0..N {
                lines = lines | mux(srcs[i].get(), one << (i + 1), none);
            }
            let sources = U::<32>::from((((1u64 << (N + 1)) - 1) & !1) as u32);
            let edges = U::<32>::from(EDGE as u32) & sources;
            let prev = self.prev.get();
            let held = self.held.get();
            let active = self.active.get();
            let pending = self.pending.get();
            let enable = self.enable.get();
            let threshold = self.threshold.get();
            // A level source asks while its line is high; an edge
            // source asks on a rising edge, or on one it holds. A
            // request goes forward when its source is not active.
            let rose = lines & !prev;
            let request = (lines & !edges) | ((rose | held) & edges);
            let forward = request & !active & sources;
            // end{gateway}
            // begin{choice}
            // The source a claim would take: a candidate is pending,
            // enabled and above the threshold, and one is taken over the
            // best so far only at a higher priority, so the lowest
            // number wins a tie.
            let mut best = U::<5>::from(0u8);
            let mut best_pr = U::<3>::from(0u8);
            for i in 0..N {
                let p = self.prio[i].get();
                let cand = pending.bit(i + 1)
                    & enable.bit(i + 1)
                    & Bit::from(p > threshold);
                let take = cand & Bit::from(p > best_pr);
                best = mux(take, U::<5>::from(i + 1), best);
                best_pr = mux(take, p, best_pr);
            }
            // end{choice}
            // begin{bus}
            // A read is answered in the cycle it is taken, and a write
            // is taken when its address and its word are both there,
            // and answered at once.
            let arh = bus.ar.head();
            let take_read = bus.r.ready() & bus.ar.peek().is_some();
            let _ = bus.ar.recv_if(bus.r.ready());
            let awh = bus.aw.head();
            let wh = bus.w.head();
            let wgo = bus.b.ready()
                & bus.aw.peek().is_some()
                & bus.w.peek().is_some();
            let _ = bus.aw.recv_if(wgo);
            let _ = bus.w.recv_if(wgo);
            let roff = arh.addr.slice::<0, 22>();
            let woff = awh.addr.slice::<0, 22>();
            let wdata = wh.data;
            let wprio = wdata.slice::<0, 3>();
            let wenable = wdata & sources;
            let wnum = wdata.slice::<0, 5>();
            // A claim takes the best source's request; a complete of a
            // number in range ends that source's service.
            let claim = take_read & Bit::from(roff == 0x20_0004);
            let complete = wgo
                & Bit::from(woff == 0x20_0004)
                & Bit::from(wdata <= N as u32);
            let claimed = mux(claim, one << (best.raw() as usize), none);
            let completed = mux(complete, one << (wnum.raw() as usize), none);
            // What a read answers, by the offset.
            let mut word = mux(
                roff == 0x1000,
                pending,
                mux(
                    roff == 0x2000,
                    enable,
                    mux(
                        roff == 0x20_0000,
                        threshold.zext::<32>(),
                        mux(roff == 0x20_0004, best.zext::<32>(), none),
                    ),
                ),
            );
            for i in 0..N {
                let at = roff == 4 * (i + 1);
                word = mux(at, self.prio[i].get().zext::<32>(), word);
            }
            if take_read.to_bool() {
                bus.r.send(LiteR {
                    data: word,
                    resp: Resp::Okay,
                });
            }
            if wgo.to_bool() {
                bus.b.send(LiteB { resp: Resp::Okay });
            }
            // end{bus}
            // begin{drives}
            self.prev.set(lines);
            self.pending
                .set(mux(rst, none, (pending & !claimed) | forward));
            self.active
                .set(mux(rst, none, (active & !completed) | forward));
            self.held
                .set(mux(rst, none, (held | rose) & edges & !forward));
            for i in 0..N {
                let at = wgo & (woff == 4 * (i + 1));
                with!(self <= {
                    at ? prio[i]: wprio,
                    rst ? prio[i]: U::<3>::from(0u8),
                });
            }
            with!(self <= {
                wgo & (woff == 0x2000) ? enable: wenable,
                wgo & (woff == 0x20_0000) ? threshold: wprio,
                rst ? {
                    enable: none,
                    threshold: U::<3>::from(0u8),
                },
            });
            self.asserted.set(mux(rst, Bit::Zero, Bit::from(best != 0)));
            irq.set(self.asserted);
            // end{drives}
        }
    }
}

/// The offset of the pending bits.
pub const PENDING: u32 = 0x1000;
/// The offset of the target's enable bits.
pub const ENABLE: u32 = 0x2000;
/// The offset of the target's threshold.
pub const THRESHOLD: u32 = 0x20_0000;
/// The offset of the target's claim and complete word.
pub const CLAIM: u32 = 0x20_0004;

/// The offset of source `i`'s priority.
pub const fn priority(i: u32) -> u32 {
    4 * i
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::axi_lite::{axi_lite, LiteAw, LiteHost, LitePort, LiteW};
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, signal, Clock, DefaultClock, Out, Running, Unit};
    use txhdl::types::{Bit, U};

    type Host = LiteHost<32, 32, 4>;

    /// A write of one word, and the wait for its response.
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

    /// A read of one word.
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

    async fn cycles(n: usize) {
        for _ in 0..n {
            DefaultClock::rising().await;
        }
    }

    /// Run `client` against a controller of three sources whose edge
    /// sources are `EDGE`, with the lines, the link and the interrupt
    /// line it gets, until it is done.
    fn run3<const EDGE: usize, F>(client: impl FnOnce(Rig) -> F)
    where
        F: std::future::Future<Output = ()>,
    {
        let link = axi_lite::<32, 32, 4>();
        let bus: LitePort<32, 32, 4> = link.per.into();
        let (rst_o, rst) = signal::<Bit, DefaultClock>();
        let (s1_o, s1) = signal::<Bit, DefaultClock>();
        let (s2_o, s2) = signal::<Bit, DefaultClock>();
        let (s3_o, s3) = signal::<Bit, DefaultClock>();
        let (irq_o, irq) = signal::<Bit, DefaultClock>();
        let done = Rc::new(RefCell::new(false));
        let d = done.clone();
        let rig = Rig {
            host: link.host,
            lines: [s1_o, s2_o, s3_o],
            irq,
        };
        let body = client(rig);
        let mut plic = Plic3::<EDGE>::default();
        let mut sim = Running::new(join2(
            plic.run(bus, (rst, [s1, s2, s3], irq_o)),
            async move {
                body.await;
                *d.borrow_mut() = true;
            },
        ));
        rst_o.set(Bit::One);
        sim.cycle();
        rst_o.set(Bit::Zero);
        for _ in 0..2000 {
            sim.cycle();
            if *done.borrow() {
                return;
            }
        }
        panic!("the client did not finish");
    }

    /// What a test holds: the link's host end, a line per source, and
    /// the interrupt line.
    struct Rig {
        host: Host,
        lines: [Out<Bit>; 3],
        irq: txhdl::comp::In<Bit>,
    }

    impl Rig {
        fn line(&self, i: usize, high: bool) {
            self.lines[i - 1].set(Bit::from_bool(high));
        }
        fn irq(&self) -> bool {
            self.irq.get().to_bool()
        }
    }

    #[test]
    fn the_highest_priority_is_claimed_first_and_the_lowest_number_wins_a_tie()
    {
        run3::<0, _>(|rig| async move {
            let h = &rig.host;
            write(h, priority(1), 1).await;
            write(h, priority(2), 3).await;
            write(h, priority(3), 3).await;
            write(h, ENABLE, 0xf).await;
            // Source 0 does not exist, so its enable bit does not stick.
            assert_eq!(read(h, ENABLE).await, 0xe);
            assert!(!rig.irq(), "nothing asks yet");
            for i in 1..=3 {
                rig.line(i, true);
            }
            cycles(3).await;
            assert_eq!(read(h, PENDING).await, 0xe);
            assert!(rig.irq(), "three sources ask");
            assert_eq!(read(h, CLAIM).await, 2);
            assert_eq!(read(h, CLAIM).await, 3);
            assert_eq!(read(h, CLAIM).await, 1);
            assert_eq!(read(h, CLAIM).await, 0, "all three are in service");
            cycles(2).await;
            assert!(!rig.irq(), "nothing left to claim");
            assert_eq!(read(h, PENDING).await, 0);
            // A priority reads back as written.
            assert_eq!(read(h, priority(2)).await, 3);
        });
    }

    #[test]
    fn a_priority_at_or_below_the_threshold_does_not_interrupt() {
        run3::<0, _>(|rig| async move {
            let h = &rig.host;
            write(h, priority(1), 2).await;
            write(h, ENABLE, 0x2).await;
            write(h, THRESHOLD, 2).await;
            assert_eq!(read(h, THRESHOLD).await, 2);
            rig.line(1, true);
            cycles(3).await;
            assert_eq!(read(h, PENDING).await, 0x2, "pending all the same");
            assert!(!rig.irq(), "at the threshold");
            assert_eq!(read(h, CLAIM).await, 0);
            write(h, THRESHOLD, 1).await;
            cycles(2).await;
            assert!(rig.irq(), "above the threshold");
            assert_eq!(read(h, CLAIM).await, 1);
            // A priority of 0 never interrupts, even at a threshold of 0.
            write(h, CLAIM, 1).await;
            write(h, THRESHOLD, 0).await;
            write(h, priority(1), 0).await;
            cycles(2).await;
            assert!(!rig.irq(), "priority 0");
            // Nor does a source that is not enabled.
            write(h, priority(1), 5).await;
            write(h, ENABLE, 0).await;
            cycles(2).await;
            assert!(!rig.irq(), "not enabled");
            assert_eq!(read(h, CLAIM).await, 0);
        });
    }

    #[test]
    fn a_claimed_source_asks_again_only_after_its_complete() {
        run3::<0, _>(|rig| async move {
            let h = &rig.host;
            write(h, priority(1), 1).await;
            write(h, ENABLE, 0x2).await;
            rig.line(1, true);
            cycles(3).await;
            assert_eq!(read(h, CLAIM).await, 1);
            // The line is still high, but the request is in service.
            cycles(3).await;
            assert_eq!(read(h, PENDING).await, 0);
            assert!(!rig.irq(), "in service");
            assert_eq!(read(h, CLAIM).await, 0);
            // A complete of a number out of range changes nothing.
            write(h, CLAIM, 9).await;
            cycles(3).await;
            assert_eq!(read(h, PENDING).await, 0);
            // The complete: a level source still high asks again.
            write(h, CLAIM, 1).await;
            cycles(3).await;
            assert_eq!(read(h, PENDING).await, 0x2);
            assert!(rig.irq(), "asks again");
            assert_eq!(read(h, CLAIM).await, 1);
            // Low now, so the complete ends it.
            rig.line(1, false);
            write(h, CLAIM, 1).await;
            cycles(3).await;
            assert_eq!(read(h, PENDING).await, 0);
            assert!(!rig.irq(), "done");
        });
    }

    /// Source 2 asks on an edge, sources 1 and 3 on a level.
    #[test]
    fn an_edge_source_asks_once_per_edge_and_a_level_source_while_high() {
        run3::<0b100, _>(|rig| async move {
            let h = &rig.host;
            for i in 1..=3 {
                write(h, priority(i), 1).await;
            }
            write(h, ENABLE, 0xe).await;
            // Both held high: each asks once and is claimed.
            rig.line(1, true);
            rig.line(2, true);
            cycles(3).await;
            assert_eq!(read(h, PENDING).await, 0x6);
            assert_eq!(read(h, CLAIM).await, 1);
            assert_eq!(read(h, CLAIM).await, 2);
            write(h, CLAIM, 1).await;
            write(h, CLAIM, 2).await;
            cycles(3).await;
            // Both lines are still high: the level source asks again,
            // and the edge source, with no new edge, does not.
            assert_eq!(read(h, PENDING).await, 0x2);
            assert_eq!(read(h, CLAIM).await, 1);
            rig.line(1, false);
            write(h, CLAIM, 1).await;
            // A pulse of one cycle on the edge source is kept.
            rig.line(2, false);
            cycles(2).await;
            rig.line(2, true);
            cycles(1).await;
            rig.line(2, false);
            cycles(3).await;
            assert_eq!(read(h, PENDING).await, 0x4, "the pulse was seen");
            assert_eq!(read(h, CLAIM).await, 2);
            // An edge during the service is held, and asks on the
            // complete.
            rig.line(2, true);
            cycles(1).await;
            rig.line(2, false);
            cycles(3).await;
            assert_eq!(read(h, PENDING).await, 0, "held, not pending");
            write(h, CLAIM, 2).await;
            cycles(3).await;
            assert_eq!(read(h, PENDING).await, 0x4, "asks on the complete");
            assert_eq!(read(h, CLAIM).await, 2);
            write(h, CLAIM, 2).await;
            cycles(3).await;
            assert_eq!(read(h, PENDING).await, 0, "only once");
            assert!(!rig.irq());
        });
    }

    /// The netlist of the eight-source controller, which the board
    /// uses: one module, a priority register per source.
    #[test]
    fn the_eight_source_controller_lowers() {
        let v = Plic8::<0>::verilog("plic");
        assert!(v.contains("module plic("), "the module");
        assert!(v.contains("prio_7"), "the eighth priority");
        assert!(v.contains("irq"), "the line out");
    }
}
