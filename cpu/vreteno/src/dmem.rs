// SPDX-License-Identifier: Apache-2.0
//! The data memory as an AXI peripheral: 1024 words at `DATA_BASE` in
//! four memories of a byte, one per lane, so that a store of a byte or
//! a half writes its lanes and reads nothing.
//!
//! It answers single-beat bursts, which is all the core makes. A read
//! goes into the memory's own register at the edge, the synchronous
//! read a block RAM has, and is answered the cycle after; a write is
//! held until its beat arrives, because AXI4 puts no identifier on the
//! write data channel. The router sends this peripheral only the
//! bursts in its range, so it checks no address.
use txhdl::comp::{Clock, DefaultClock, Mem, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::{Answer, PerReq, Resp, R, W};

/// Words of data memory.
pub const DMEM_WORDS: usize = 1024;

#[derive(Trace, Default)]
pub struct Dmem<const I: usize> {
    pub lane0: Mem<U<8>, DMEM_WORDS>,
    pub lane1: Mem<U<8>, DMEM_WORDS>,
    pub lane2: Mem<U<8>, DMEM_WORDS>,
    pub lane3: Mem<U<8>, DMEM_WORDS>,
    /// The word a read landed in, whose identifier it answers, and
    /// whether it is to be answered.
    pub word: Reg<U<32>>,
    pub rid: Reg<U<I>>,
    pub answer: Reg<Bit>,
    /// A write taken and waiting for its beat: where it goes and which
    /// identifier answers it.
    pub pend: Reg<U<1>>,
    pub paddr: Reg<U<10>>,
    pub pid: Reg<U<I>>,
}

impl<const I: usize> Dmem<I> {
    /// A memory holding `bytes` from its base before the first cycle:
    /// the constants a compiled program reads. Nothing on this machine
    /// can put them there at run time, because the instruction memory
    /// a program is loaded into is not on the bus and a load never
    /// reaches it, so what a program reads has to be here already.
    /// The bytes go a lane each, as an address's low two bits choose.
    pub fn with(bytes: &[u8]) -> Self {
        // A drive through `at` is deferred to the edge, as a register's
        // is, so the lanes are built whole and handed to `Mem::with`.
        let words = bytes.len().div_ceil(4).min(DMEM_WORDS);
        let lane = |k: usize| -> Vec<U<8>> {
            (0..words)
                .map(|w| U::from(*bytes.get(w * 4 + k).unwrap_or(&0)))
                .collect()
        };
        Dmem {
            lane0: Mem::with(&lane(0)),
            lane1: Mem::with(&lane(1)),
            lane2: Mem::with(&lane(2)),
            lane3: Mem::with(&lane(3)),
            ..Default::default()
        }
    }

    /// A word of the memory, for the run and the test to look at.
    pub fn data_word(&self, at: usize) -> u32 {
        let lane = |m: &Mem<U<8>, DMEM_WORDS>| m.read(at).raw() as u32;
        lane(&self.lane0)
            | lane(&self.lane1) << 8
            | lane(&self.lane2) << 16
            | lane(&self.lane3) << 24
    }
}

#[lower]
impl<const I: usize> Unit for Dmem<I> {
    async fn run(
        &mut self,
        (req, wd): (Rx<PerReq<32, I>>, Rx<W<32, 4>>),
        (ans, rb): (Tx<Answer<I>>, Tx<R<32, I>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let q = req.head();
            let qoff = req.peek().is_some();
            let held = self.pend.get() == 1;
            let queued = self.answer.to_bool();
            // The word this burst names, within the memory.
            let at = q.addr.slice::<2, 10>();
            // A read is taken when the register it lands in is free or
            // is being emptied this cycle; a write is taken when no
            // other write is waiting for its beat.
            let send = queued & rb.ready();
            let take_read = qoff & q.read & !held & (!queued | rb.ready());
            let take_write = qoff & !q.read & !held;
            let _ = req.recv_if(take_read | take_write);
            let wh = wd.head();
            let wgo = held & wd.peek().is_some() & ans.ready();
            let _ = wd.recv_if(wgo);
            let data = wh.data;
            let strb = wh.strb;
            let to = self.paddr.get();
            with!(self <= {
                wgo & strb.bit(0) ? lane0.at(to): data.slice::<0, 8>(),
                wgo & strb.bit(1) ? lane1.at(to): data.slice::<8, 8>(),
                wgo & strb.bit(2) ? lane2.at(to): data.slice::<16, 8>(),
                wgo & strb.bit(3) ? lane3.at(to): data.slice::<24, 8>(),
                take_write ? {
                    pend: U::<1>::from(1u8),
                    paddr: at,
                    pid: q.id,
                },
                wgo ? pend: U::<1>::from(0u8),
                take_read ? {
                    word: self
                        .lane3
                        .read(at)
                        .concat::<_, 16>(self.lane2.read(at))
                        .concat::<_, 24>(self.lane1.read(at))
                        .concat::<_, 32>(self.lane0.read(at)),
                    rid: q.id,
                    answer: Bit::One,
                } else {
                    send ? answer: Bit::Zero,
                },
            });
            if send.to_bool() {
                rb.send(R {
                    id: self.rid.get(),
                    data: self.word.get(),
                    resp: Resp::Okay,
                    last: Bit::One,
                });
            }
            if wgo.to_bool() {
                ans.send(Answer {
                    id: self.pid.get(),
                    resp: Resp::Okay,
                });
            }
        }
    }
}
