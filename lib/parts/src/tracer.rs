// SPDX-License-Identifier: Apache-2.0
//! A trace buffer: the last `N` things that happened, kept where a
//! host can read them after they have stopped happening.
//!
//! A design on a board says almost nothing. Four LEDs and a serial
//! line are what a program has, and neither answers the question a
//! post-mortem asks, which is what the machine did in the cycles
//! before it stopped. The core already computes the answer: it drives
//! the instruction it retired and the register it wrote, every cycle,
//! and on the board those wires go nowhere (issue 240).
//!
//! This part takes a word a cycle and keeps the last `N` of them in a
//! ring. When the ring is full the oldest goes, so what is held is
//! always the most recent window. A host reads the window out over
//! AXI-Lite, oldest first.
//!
//! | Offset | Name | What it is |
//! |---|---|---|
//! | `0x00` | `ctrl` | run, and freeze when `halt` rises |
//! | `0x04` | `count` | entries held, read only |
//! | `0x08` | `cursor` | which held entry a read returns |
//! | `0x10` | `word0` | bits 31 to 0 of that entry |
//! | `0x14` | `word1` | bits 63 to 32 |
//! | `0x18` | `word2` | bits 95 to 64 |
//! | `0x1c` | `word3` | bits 127 to 96 |
//!
//! Freezing is the point of the `halt` input. A program that stops on
//! a breakpoint, or a core that halts on `ebreak`, leaves a window
//! that a running buffer would overwrite with whatever the machine
//! does next; with `CTRL_FREEZE` set, the first cycle `halt` is high
//! stops the buffer and the window stands until a host starts it
//! again.
//!
//! An entry is 128 bits because a retirement is: a program counter, an
//! instruction, the register written and the word written come to a
//! hundred and one bits, and a width that is a whole number of bus
//! words is one the readout can walk without shifting.
use txhdl::comp::{Clock, DefaultClock, In, Mem, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};

use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteB, LitePort, LiteR};

/// `ctrl` bit 0: the buffer takes what it is offered.
pub const CTRL_RUN: u32 = 1;
/// `ctrl` bit 1: the first cycle of `halt` clears `run`.
pub const CTRL_FREEZE: u32 = 2;

/// The offset of word `k` of the entry the cursor names.
pub const fn word(k: u32) -> u32 {
    0x10 + 4 * k
}

// begin{state}
/// A ring of `N` entries of 128 bits, read out over AXI-Lite.
///
/// `N` is a power of two, since the ring wraps by masking.
#[derive(Trace, Default)]
pub struct Tracer<const N: usize> {
    /// Run, and freeze on `halt`.
    pub ctrl: Reg<U<2>>,
    /// The slot the next entry goes in.
    pub head: Reg<U<16>>,
    /// How many entries are held, up to `N`.
    pub count: Reg<U<16>>,
    /// Which held entry a read returns, counting from the oldest.
    pub cursor: Reg<U<16>>,
    /// The ring itself.
    pub ring: Mem<U<128>, N>,
}
// end{state}

// begin{run}
#[lower]
impl<const N: usize> Unit for Tracer<N> {
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        (take, entry, halt): (In<Bit>, In<U<128>>, In<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let ctrl = self.ctrl.get();
            let head = self.head.get();
            let count = self.count.get();
            let cursor = self.cursor.get();
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
            let running = ctrl.bit(0);
            let freezing = ctrl.bit(1) & halt.get();
            let mask = U::<16>::from((N - 1) as u32);
            // What is held runs from the oldest to the slot before the
            // head, so the cursor counts from there.
            let oldest = (head - count) & mask;
            let slot = (oldest + cursor) & mask;
            let held = self.ring.read(slot);
            let storing = running & take.get() & !freezing;
            let full = count == U::<16>::from(N as u32);
            let answer = select!(rsel.raw() => {
                0 => ctrl.zext::<32>(),
                1 => count.zext::<32>(),
                2 => cursor.zext::<32>(),
                4 => held.slice::<0, 32>(),
                5 => held.slice::<32, 32>(),
                6 => held.slice::<64, 32>(),
                7 => held.slice::<96, 32>(),
                _ => U::<32>::from(0u8),
            });
            if storing.to_bool() {
                self.ring.at(head).set(entry.get());
            }
            with!(self <= {
                wgo & (wsel == 0) ? ctrl: written.slice::<0, 2>(),
                // A host that starts the buffer starts a new window.
                wgo & (wsel == 0) & written.bit(0) & !running ? count:
                    U::<16>::from(0u8),
                wgo & (wsel == 0) & written.bit(0) & !running ? head:
                    U::<16>::from(0u8),
                wgo & (wsel == 2) ? cursor: written.slice::<0, 16>(),
                // The freeze is a level that clears the run bit, so a
                // host reads why the buffer stopped in `ctrl`.
                freezing ? ctrl: ctrl & U::<2>::from(2u8),
                storing ? head: (head + 1) & mask,
                storing & !full ? count: count + 1,
            });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::axi_lite::{axi_lite, LiteAw, LiteHost, LiteW};
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, signal, Out, Running};

    type Host = LiteHost<32, 32, 4>;

    /// A ring of eight entries, which is what the tests use.
    type Ring = Tracer<8>;

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

    /// What a test holds: the link's host end, and the three wires the
    /// design being watched would drive.
    struct Rig {
        host: Host,
        take: Out<Bit>,
        entry: Out<U<128>>,
        halt: Out<Bit>,
    }

    impl Rig {
        /// Offer one entry, on the cycle after this call.
        async fn offer(&self, v: u32) {
            self.entry.set(U::<128>::from(v));
            self.take.set(Bit::One);
            DefaultClock::rising().await;
            self.take.set(Bit::Zero);
        }
        /// Read the low word of the entry the cursor names.
        async fn at(&self, i: u32) -> u32 {
            write(&self.host, 8, i).await;
            read(&self.host, word(0)).await
        }
    }

    fn run<F>(client: impl FnOnce(Rig) -> F)
    where
        F: std::future::Future<Output = ()>,
    {
        let link = axi_lite::<32, 32, 4>();
        let bus: LitePort<32, 32, 4> = link.per.into();
        let (take_o, take) = signal::<Bit, DefaultClock>();
        let (entry_o, entry) = signal::<U<128>, DefaultClock>();
        let (halt_o, halt) = signal::<Bit, DefaultClock>();
        let done = Rc::new(RefCell::new(false));
        let d = done.clone();
        let body = client(Rig {
            host: link.host,
            take: take_o,
            entry: entry_o,
            halt: halt_o,
        });
        let mut tracer = Ring::default();
        let mut sim = Running::new(join2(
            async move {
                body.await;
                *d.borrow_mut() = true;
            },
            tracer.run(bus, (take, entry, halt)),
        ));
        for _ in 0..20000 {
            sim.cycle();
            if *done.borrow() {
                return;
            }
        }
        panic!("the client did not finish");
    }

    #[test]
    fn what_is_held_is_the_last_n_entries() {
        run(|rig| async move {
            write(&rig.host, 0, CTRL_RUN).await;
            for v in 1..=12u32 {
                rig.offer(v).await;
            }
            assert_eq!(read(&rig.host, 4).await, 8, "the ring is full");
            // Twelve offered, eight held: the oldest of them is the
            // fifth, and the newest is the twelfth.
            assert_eq!(rig.at(0).await, 5);
            assert_eq!(rig.at(7).await, 12);
        });
    }

    #[test]
    fn fewer_than_a_ring_are_held_oldest_first() {
        run(|rig| async move {
            write(&rig.host, 0, CTRL_RUN).await;
            for v in 1..=3u32 {
                rig.offer(v).await;
            }
            assert_eq!(read(&rig.host, 4).await, 3);
            assert_eq!(rig.at(0).await, 1);
            assert_eq!(rig.at(1).await, 2);
            assert_eq!(rig.at(2).await, 3);
        });
    }

    #[test]
    fn a_halt_freezes_the_window() {
        run(|rig| async move {
            write(&rig.host, 0, CTRL_RUN | CTRL_FREEZE).await;
            for v in 1..=3u32 {
                rig.offer(v).await;
            }
            rig.halt.set(Bit::One);
            DefaultClock::rising().await;
            for v in 4..=9u32 {
                rig.offer(v).await;
            }
            assert_eq!(read(&rig.host, 4).await, 3, "nothing after the halt");
            let ctrl = read(&rig.host, 0).await;
            assert_eq!(ctrl & CTRL_RUN, 0, "and the ctrl word says so");
            assert_eq!(rig.at(0).await, 1);
            assert_eq!(rig.at(2).await, 3);
        });
    }

    #[test]
    fn a_start_opens_a_new_window() {
        run(|rig| async move {
            write(&rig.host, 0, CTRL_RUN).await;
            for v in 1..=5u32 {
                rig.offer(v).await;
            }
            write(&rig.host, 0, 0).await;
            write(&rig.host, 0, CTRL_RUN).await;
            for v in 100..=102u32 {
                rig.offer(v).await;
            }
            assert_eq!(read(&rig.host, 4).await, 3, "only the new window");
            assert_eq!(rig.at(0).await, 100);
        });
    }

    #[test]
    fn a_stopped_buffer_takes_nothing() {
        run(|rig| async move {
            for v in 1..=4u32 {
                rig.offer(v).await;
            }
            assert_eq!(read(&rig.host, 4).await, 0, "nothing was taken");
        });
    }
}
