// SPDX-License-Identifier: Apache-2.0
//! The platform-level interrupt controller: `Plic1` to `Plic8`, one
//! type per count of sources, each written out by `plic!`, since the
//! lowering reads a body and not a loop over ports.
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
use txhdl::plic;

plic!(Plic1, 1);
plic!(Plic2, 2);
plic!(Plic3, 3);
plic!(Plic4, 4);
plic!(Plic5, 5);
plic!(Plic6, 6);
plic!(Plic7, 7);
plic!(Plic8, 8);

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
            plic.run(bus, (rst, s1, s2, s3, irq_o)),
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
        assert!(v.contains("prio8"), "the eighth priority");
        assert!(v.contains("irq"), "the line out");
    }
}
