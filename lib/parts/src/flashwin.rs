// SPDX-License-Identifier: Apache-2.0
//! A read-only window onto an SPI flash: a read on the bus becomes a
//! fast read on the wires, and the word comes back.
//!
//! The SPI master in `spi` is what a program drives byte by byte. This
//! is the other way of reaching the same chip: an address range in
//! which an ordinary read is answered with what the flash holds there,
//! with no registers to drive and nothing for software to sequence. A
//! bootloader reads a program out of it, and a core that fetches from
//! the bus can run from it, without a resynthesis to change what is
//! there (issue 146).
//!
//! A read at offset `a` sends the fast read command `0x0b`, the three
//! bytes of `a` with its low two bits cleared, one byte the chip
//! throws away, and then takes four bytes, which are the word, lowest
//! address in the lowest byte. Nine bytes on the wires for four on the
//! bus: the window trades time for needing no software at all, and a
//! cache in front of it is the answer if that matters.
//!
//! The wires are mode 0, which is what every flash answers a fast read
//! in: the clock idles low, the chip takes a bit on the rising edge
//! and the master moves one on the falling. The select is held low for
//! the whole of a read and released after it, so each read is a
//! command of its own.
//!
//! A write is answered `SLVERR`. The window is read only, which is
//! what makes it safe to fetch from: nothing on the bus can change
//! what the instruction stream is reading.
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};

use crate::bus::axi::{Answer, PerPort, Resp, R};

// begin{state}
/// The window. `DIV` sets the clock: a half of a bit takes `DIV + 1`
/// cycles, so the flash is clocked at the system's rate over
/// `2 * (DIV + 1)`. `I` is the width of the AXI identifier.
#[derive(Trace, Default)]
pub struct FlashWin<const DIV: usize, const I: usize> {
    /// A read is on the wires.
    pub busy: Reg<Bit>,
    /// Cycles since the clock last turned, which is the same shape
    /// the master in `spi` has.
    pub tick: Reg<U<8>>,
    /// Which half of a bit the clock is in: low, then high.
    pub half: Reg<Bit>,
    /// Half edges within the byte, sixteen to a byte.
    pub count: Reg<U<5>>,
    /// The byte going out, top bit first.
    pub txb: Reg<U<8>>,
    /// The byte coming in, a bit at a time from the top.
    pub rxb: Reg<U<8>>,
    /// Which byte of the nine the read is on: the command, three of
    /// address, the one thrown away, and four of data.
    pub phase: Reg<U<4>>,
    /// The address asked for, with its low two bits cleared.
    pub addr: Reg<U<24>>,
    /// The word being built from the four bytes as they arrive.
    pub word: Reg<U<32>>,
    /// The identifier that answers this read.
    pub rid: Reg<U<I>>,
    /// Whether an answer is waiting for room on the channel.
    pub ready: Reg<Bit>,
    /// A write taken and waiting for its beat. The window is read
    /// only, so the beat is dropped and the answer is an error.
    pub wpend: Reg<U<1>>,
    /// The identifier that write is answered under.
    pub wid: Reg<U<I>>,
}
// end{state}

// begin{run}
#[lower]
impl<const DIV: usize, const I: usize> Unit for FlashWin<DIV, I> {
    async fn run(
        &mut self,
        bus: PerPort<32, 32, 4, I>,
        (rst, miso, sclk, mosi, cs_n): (
            In<Bit>,
            In<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let div = U::<8>::from(DIV as u32);
            let busy = self.busy.get();
            let tick = self.tick.get();
            let half = self.half.get();
            let count = self.count.get();
            let txb = self.txb.get();
            let rxb = self.rxb.get();
            let phase = self.phase.get();
            let waiting = self.ready.get();
            let held = self.wpend.get() == 1;
            // A half of a bit has passed. The clock idles low and the
            // chip takes its bit as the clock rises, so the master
            // takes the chip's on the same edge and moves its own on
            // the fall, which is mode 0 and the only mode here.
            let strobe = busy & (tick == div);
            let taking = strobe & !half;
            let moving = strobe & half;
            let last = strobe & (count == U::<5>::from(15u8));
            // The bus. A read is taken when nothing is on the wires
            // and no answer is waiting; a write is taken when no other
            // write is waiting for its beat.
            let q = bus.req.head();
            let q_off = bus.req.peek().is_some();
            let take_read = q_off & q.read & !busy & !waiting & !held;
            let take_write = q_off & !q.read & !held;
            let _ = bus.req.recv_if(take_read | take_write);
            let w_go = held & bus.w.peek().is_some() & bus.ans.ready();
            let _ = bus.w.recv_if(w_go);
            let send = waiting & bus.r.ready();
            // The byte this phase puts on the wires: the command, then
            // the address from the top down, then nothing, since the
            // chip is doing the talking from there.
            // The byte loaded when a phase ends is the next phase's,
            // so the select is on the phase after this one. The first
            // byte, the command, is loaded when the read is taken.
            let at = self.addr.get();
            let next = phase + U::<4>::from(1u8);
            let out_byte = select!(next.raw() => {
                1 => at.slice::<16, 8>(),
                2 => at.slice::<8, 8>(),
                3 => at.slice::<0, 8>(),
                _ => U::<8>::from(0u8),
            });
            // The four bytes of data land in the word lowest address
            // first, which is how a little-endian machine reads them.
            let got = rxb.zext::<32>();
            let done = last & (phase == U::<4>::from(8u8));
            with!(self <= {
                rst.get() ? {
                    busy: Bit::Zero,
                    ready: Bit::Zero,
                    wpend: U::<1>::from(0u8),
                } else {
                    take_read ? {
                        busy: Bit::One,
                        addr: q.addr.slice::<0, 24>()
                            & U::<24>::from(0xff_fffcu32),
                        rid: q.id,
                        phase: U::<4>::from(0u8),
                        txb: U::<8>::from(0x0bu8),
                        tick: U::<8>::from(0u8),
                        half: Bit::Zero,
                        count: U::<5>::from(0u8),
                        word: U::<32>::from(0u32),
                    },
                    take_write ? {
                        wpend: U::<1>::from(1u8),
                        wid: q.id,
                    },
                    w_go ? wpend: U::<1>::from(0u8),
                    busy ? tick: mux(strobe, U::<8>::from(0u8), tick + 1),
                    strobe ? {
                        half: !half,
                        count: count + 1,
                    },
                    taking ? rxb: (rxb << 1) | miso.get().zext::<8>(),
                    moving ? txb: txb << 1,
                    last & (phase == U::<4>::from(5u8)) ? word: got,
                    last & (phase == U::<4>::from(6u8)) ?
                        word: self.word.get() | (got << 8),
                    last & (phase == U::<4>::from(7u8)) ?
                        word: self.word.get() | (got << 16),
                    last & (phase == U::<4>::from(8u8)) ?
                        word: self.word.get() | (got << 24),
                    last & !done ? {
                        phase: phase + 1,
                        txb: out_byte,
                        count: U::<5>::from(0u8),
                        half: Bit::Zero,
                    },
                    done ? {
                        busy: Bit::Zero,
                        half: Bit::Zero,
                        ready: Bit::One,
                    },
                    send ? ready: Bit::Zero,
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
            if w_go.to_bool() {
                bus.ans.send(Answer {
                    id: self.wid.get(),
                    resp: Resp::SlvErr,
                });
            }
            // The select is low for the whole of a read and goes up
            // when the ninth byte is done, so each read is a command
            // the chip sees whole.
            sclk.set(busy & half);
            mosi.set(txb.bit(7));
            cs_n.set(!busy);
        }
    }
}
// end{run}
