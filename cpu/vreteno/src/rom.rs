// SPDX-License-Identifier: Apache-2.0
//! The boot memory as an AXI peripheral, readable and not writable:
//! the same 1024 words the core fetches from inside itself, on the bus
//! at address zero, so that a load can read a constant that sits next
//! to the code, and a debugger or a host on the bus can read the
//! program that is there.
//!
//! The core keeps its own copy for the fetch, which reads it in the
//! same cycle; this copy answers the bus, a cycle after a read is
//! taken, as the data memory does. Both are initialised with the same
//! image by the netlist, so they cannot disagree, and neither can be
//! written at run time: a write burst is taken, every beat of it up to
//! the last consumed so the channel does not jam (issue 1208), and
//! answered `SlvErr` once, which is what a
//! peripheral that was reached and refused says. That settles the
//! question of what a fetch sees when a write lands on the same
//! address in the same cycle: nothing lands.
//!
//! A read burst of incrementing words is answered a beat a cycle, each
//! word read as the one before it goes, as the data memory answers
//! one. The core makes single beats, but a host on the bus that asks
//! for a burst here is owed every beat it asked for: the scanout's
//! fetch, out of a reset, asked for a line of sixteen-beat bursts at
//! zero, was answered one beat, and waited for the rest for good
//! (issue 1193). The router sends it only the bursts in its range, so
//! it checks no address.
use txhdl::comp::{Clock, DefaultClock, Mem, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::{Answer, PerPort, Resp, R};

use crate::core::IMEM_WORDS;

#[derive(Trace, Default)]
pub struct Rom<const I: usize> {
    /// The program, a word per address, as the core's own copy holds it.
    pub words: Mem<U<32>, IMEM_WORDS>,
    /// The word a read landed in, whose identifier it answers, and
    /// whether it is to be answered.
    pub word: Reg<U<32>>,
    pub rid: Reg<U<I>>,
    pub answer: Reg<Bit>,
    /// The beats of a read burst left after the one in `word`, and
    /// the word the next of them reads.
    pub rleft: Reg<U<8>>,
    pub raddr: Reg<U<10>>,
    /// A write taken and waiting for its beat, so that the beat can be
    /// consumed and the burst refused: whether one is, and which
    /// identifier answers it.
    pub pend: Reg<U<1>>,
    pub pid: Reg<U<I>>,
}

impl<const I: usize> Rom<I> {
    /// A memory holding `program` before the first cycle: the same
    /// words `Vreteno::with` puts in the core.
    pub fn with(program: &[u32]) -> Self {
        let words: Vec<U<32>> = program.iter().map(|&w| U::from(w)).collect();
        Rom {
            words: Mem::with(&words),
            ..Default::default()
        }
    }
}

#[lower]
impl<const I: usize> Unit for Rom<I> {
    async fn run(&mut self, bus: PerPort<32, 32, 4, I>, _o: ()) {
        loop {
            DefaultClock::rising().await;
            let q = bus.req.head();
            let qoff = bus.req.peek().is_some();
            let held = self.pend.get() == 1;
            let queued = self.answer.to_bool();
            // The word this burst names, within the memory.
            let at = q.addr.slice::<2, 10>();
            // A read is taken when the register it lands in is free or
            // is sending a burst's last beat this cycle; a write is
            // taken when no other write is waiting for its beat.
            let send = queued & bus.r.ready();
            let more = Bit::from(self.rleft.get() != 0);
            let next = send & more;
            let take_read =
                qoff & q.read & !held & (!queued | (bus.r.ready() & !more));
            let nat = self.raddr.get();
            let take_write = qoff & !q.read & !held;
            let _ = bus.req.recv_if(take_read | take_write);
            // The beat of a refused write is taken and dropped, so that
            // the write data channel is not left holding it.
            // Every beat of a refused write burst is taken, and the
            // burst answered once, after its last (issue 1208).
            let wlast = bus.w.head().last;
            let wgo =
                held & bus.w.peek().is_some() & (bus.ans.ready() | !wlast);
            let _ = bus.w.recv_if(wgo);
            with!(self <= {
                take_write ? {
                    pend: U::<1>::from(1u8),
                    pid: q.id,
                },
                wgo & wlast ? pend: U::<1>::from(0u8),
                take_read ? {
                    word: self.words.read(at),
                    rid: q.id,
                    answer: Bit::One,
                    rleft: q.len,
                    raddr: at + 1,
                } else {
                    next ? {
                        word: self.words.read(nat),
                        rleft: self.rleft.get() - 1,
                        raddr: nat + 1,
                    } else {
                        send ? answer: Bit::Zero,
                    },
                },
            });
            if send.to_bool() {
                bus.r.send(R {
                    id: self.rid.get(),
                    data: self.word.get(),
                    resp: Resp::Okay,
                    last: !more,
                });
            }
            if (wgo & wlast).to_bool() {
                bus.ans.send(Answer {
                    id: self.pid.get(),
                    resp: Resp::SlvErr,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, Running};
    use txhdl_parts::bus::axi::{
        axi_units, host_end, AxiHost, AxiPer, Host, Rd,
    };

    /// A read burst of sixteen beats, which is how the scanout's fetch
    /// asks for a line, gets sixteen words in order with the last one
    /// marked, and a single beat after it gets its one word as before
    /// (issue 1193).
    #[test]
    fn a_read_burst_gets_every_beat() {
        let program: Vec<u32> = (0..64u32).map(|i| 0x1000 + i).collect();
        let mut rom = Rom::<2>::with(&program);
        let u = axi_units::<32, 32, 4, 2>();
        let host: Host<32, 32, 4, 2, 4> = host_end(u.host_client);
        let bus = PerPort::from(u.per_client);
        let done = Rc::new(RefCell::new(false));
        let d = done.clone();
        let client = async move {
            let got = host.read(Rd::at(0x40u32, 16)).await.done().await;
            assert_eq!(got.resp, Resp::Okay);
            let raw: Vec<u128> = got.data.iter().map(|x| x.raw()).collect();
            let want: Vec<u128> = (16..32).map(|i| 0x1000 + i).collect();
            assert_eq!(raw, want, "the sixteen words, in order");
            let got = host.read(Rd::at(0x8u32, 1)).await.done().await;
            assert_eq!(got.data.len(), 1);
            assert_eq!(got.data[0].raw(), 0x1002);
            *d.borrow_mut() = true;
        };
        let mut tracker = AxiHost::<32, 32, 4, 2, 4>::default();
        let mut per = AxiPer::<32, 32, 4, 2>::default();
        let mut sim = Running::new(join2(
            client,
            join2(
                tracker.run(u.host_in, u.host_out),
                join2(per.run(u.per_in, u.per_out), rom.run(bus, ())),
            ),
        ));
        for _ in 0..400 {
            sim.cycle();
            if *done.borrow() {
                break;
            }
        }
        assert!(*done.borrow(), "every beat of both reads came back");
    }
}
