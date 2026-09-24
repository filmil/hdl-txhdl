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
//! written at run time: a write burst is taken, its beat consumed so
//! the channel does not jam, and answered `SlvErr`, which is what a
//! peripheral that was reached and refused says. That settles the
//! question of what a fetch sees when a write lands on the same
//! address in the same cycle: nothing lands.
//!
//! It answers single-beat bursts, which is all the core makes. The
//! router sends it only the bursts in its range, so it checks no
//! address.
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
            // is being emptied this cycle; a write is taken when no
            // other write is waiting for its beat.
            let send = queued & bus.r.ready();
            let take_read = qoff & q.read & !held & (!queued | bus.r.ready());
            let take_write = qoff & !q.read & !held;
            let _ = bus.req.recv_if(take_read | take_write);
            // The beat of a refused write is taken and dropped, so that
            // the write data channel is not left holding it.
            let wgo = held & bus.w.peek().is_some() & bus.ans.ready();
            let _ = bus.w.recv_if(wgo);
            with!(self <= {
                take_write ? {
                    pend: U::<1>::from(1u8),
                    pid: q.id,
                },
                wgo ? pend: U::<1>::from(0u8),
                take_read ? {
                    word: self.words.read(at),
                    rid: q.id,
                    answer: Bit::One,
                } else {
                    send ? answer: Bit::Zero,
                },
            });
            if send.to_bool() {
                bus.r.send(R {
                    id: self.rid.get(),
                    data: self.word.get(),
                    resp: Resp::Okay,
                    last: Bit::One,
                });
            }
            if wgo.to_bool() {
                bus.ans.send(Answer {
                    id: self.pid.get(),
                    resp: Resp::SlvErr,
                });
            }
        }
    }
}
