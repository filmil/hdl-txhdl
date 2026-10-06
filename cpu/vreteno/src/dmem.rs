// SPDX-License-Identifier: Apache-2.0
//! The data memory as an AXI peripheral: `W` words, 1024 at
//! `DATA_BASE` on the board, in four memories of a byte, one per lane,
//! so that a store of a byte or a half writes its lanes and reads
//! nothing. The board's stack memory is the same unit with 16384 words
//! (issue 1278).
//!
//! A read goes into the memory's own register at the edge, the
//! synchronous read a block RAM has, and is answered the cycle after.
//! A read burst of incrementing words is answered a beat a cycle, each
//! word read as the one before it goes, which is what the core's
//! instruction cache fills its lines with (issue 1021). A write is
//! held until its beat arrives, because AXI4 puts no identifier on the
//! write data channel. A write burst's beats go to consecutive words,
//! and the burst is answered once, after its last beat, so that a burst
//! host's beats never stay in the write channel behind an early answer
//! (issue 1208). The router sends this peripheral only the bursts in
//! its range, so it checks no address.
use txhdl::comp::{mux, Clock, DefaultClock, Mem, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::{Answer, PerPort, Resp, R};

/// Words of data memory.
pub const DMEM_WORDS: usize = 1024;

/// `I` is the width of the link's identifier, `W` the words, a power
/// of two, and `AW` the bits that number them, so that `W` is
/// `1 << AW`. The data memory is the defaults.
#[derive(Trace, Default)]
pub struct Dmem<
    const I: usize,
    const AW: usize = 10,
    const W: usize = DMEM_WORDS,
> {
    pub lane0: Mem<U<8>, W>,
    pub lane1: Mem<U<8>, W>,
    pub lane2: Mem<U<8>, W>,
    pub lane3: Mem<U<8>, W>,
    /// The word a read landed in, whose identifier it answers, and
    /// whether it is to be answered.
    pub word: Reg<U<32>>,
    pub rid: Reg<U<I>>,
    pub answer: Reg<Bit>,
    /// The beats of a read burst left after the one in `word`, and
    /// the word the next of them reads.
    pub rleft: Reg<U<8>>,
    pub raddr: Reg<U<AW>>,
    /// A write taken and waiting for its beat: where it goes and which
    /// identifier answers it.
    pub pend: Reg<U<1>>,
    pub paddr: Reg<U<AW>>,
    pub pid: Reg<U<I>>,
}

impl<const I: usize, const AW: usize, const W: usize> Dmem<I, AW, W> {
    /// A memory holding `bytes` from its base before the first cycle:
    /// a compiled program's initialised data. Its constants live in
    /// the boot memory beside the code, which is on the bus read-only
    /// (issue 268); what a program writes lives here, and the image
    /// carries the initial bytes. The bytes go a lane each, as an
    /// address's low two bits choose.
    pub fn with(bytes: &[u8]) -> Self {
        // A drive through `at` is deferred to the edge, as a register's
        // is, so the lanes are built whole and handed to `Mem::with`.
        let words = bytes.len().div_ceil(4).min(W);
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
        let lane = |m: &Mem<U<8>, W>| m.read(at).raw() as u32;
        lane(&self.lane0)
            | lane(&self.lane1) << 8
            | lane(&self.lane2) << 16
            | lane(&self.lane3) << 24
    }
}

#[lower]
impl<const I: usize, const AW: usize, const W: usize> Unit for Dmem<I, AW, W> {
    async fn run(&mut self, bus: PerPort<32, 32, 4, I>, _o: ()) {
        loop {
            DefaultClock::rising().await;
            let q = bus.req.head();
            let qoff = bus.req.peek().is_some();
            let held = self.pend.get() == 1;
            let queued = self.answer.to_bool();
            // The word this burst names, within the memory.
            let at = q.addr.slice::<2, AW>();
            // A read is taken when the register it lands in is free or
            // is sending a burst's last beat this cycle; a write is
            // taken when no other write is waiting for its beat.
            let send = queued & bus.r.ready();
            let more = Bit::from(self.rleft.get() != 0);
            let next = send & more;
            let take_read =
                qoff & q.read & !held & (!queued | (bus.r.ready() & !more));
            let nat = self.raddr.get();
            // One read of the four lanes a cycle, at one address: a new
            // read's word, or a burst's next. Two reads at two addresses
            // were two read ports beside the write's, which no block RAM
            // has, so Vivado built the memory from LUTs: 35 K of them for
            // the 64 KiB stack memory (issue 1301).
            let ra = mux(take_read, at, nat);
            let take_write = qoff & !q.read & !held;
            let _ = bus.req.recv_if(take_read | take_write);
            // A write burst's beats go to consecutive words, and only
            // its last is answered, so only the last waits for room.
            let wh = bus.w.head();
            let wlast = wh.last;
            let wgo =
                held & bus.w.peek().is_some() & (bus.ans.ready() | !wlast);
            let _ = bus.w.recv_if(wgo);
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
                wgo & !wlast ? paddr: to + 1,
                wgo & wlast ? pend: U::<1>::from(0u8),
                take_read | next ? word: self
                    .lane3
                    .read(ra)
                    .concat::<_, 16>(self.lane2.read(ra))
                    .concat::<_, 24>(self.lane1.read(ra))
                    .concat::<_, 32>(self.lane0.read(ra)),
                take_read ? {
                    rid: q.id,
                    answer: Bit::One,
                    rleft: q.len,
                    raddr: at + 1,
                } else {
                    next ? {
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
                    resp: Resp::Okay,
                });
            }
        }
    }
}
