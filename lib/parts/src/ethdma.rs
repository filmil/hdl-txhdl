// SPDX-License-Identifier: Apache-2.0
//! Between the words an engine moves and the bytes a frame counts.
//!
//! [`crate::dma::LineFetch`] and [`crate::dma::LineStore`] move
//! 32-bit words, because that is what the bus moves. An Ethernet
//! frame is a count of bytes, and not usually a whole number of
//! words: anything from 64 to 1518 bytes, so its last word holds
//! one, two or three real bytes as often as four.
//!
//! These two units are that difference, and nothing else. Neither
//! touches the bus and neither knows where the frame is in memory.
//! `FrameOut` takes words from a fetch engine and gives the
//! transmitter bytes, the last of them marked. `FrameIn` takes the
//! receiver's bytes and gives a store engine words.
//!
//! # Why the count is carried rather than inferred
//!
//! A word tells you nothing about how many of its bytes are real, so
//! the byte count has to arrive beside the words. On the way out it
//! comes from `tx_length`, which the driver wrote. On the way in it
//! comes from the receiver, which knows the frame's length before it
//! offers the first byte of it, because it takes a whole frame and
//! checks it before offering any of it.
//!
//! Getting that wrong is quiet rather than loud. Three bytes too many
//! puts whatever the channel was carrying on the wire, or into memory
//! past the end of the frame, and the frame still looks well formed
//! going past.
use txhdl::comp::{mux, Clock, DefaultClock, In, Mem, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

use crate::eth::{EthByte, FRAME_MAX};

// begin{out}
/// Words from a fetch engine, bytes to the transmitter.
///
/// A frame is `bytes` long. The unit takes one word at a time, emits
/// its bytes lowest first, and marks the last byte of the frame
/// rather than the last byte of the last word, so the three bytes
/// that may sit above it are never sent.
#[derive(Trace, Default)]
pub struct FrameOut {
    /// The frame's length in bytes, as the run was started with.
    pub want: Reg<U<16>>,
    /// Bytes emitted so far.
    pub pos: Reg<U<16>>,
    /// The word being emitted, shifted down as its bytes go.
    pub word: Reg<U<32>>,
    /// How many of its bytes are still to go.
    pub have: Reg<U<3>>,
    /// Whether a frame is going.
    pub run: Reg<Bit>,
}

#[lower]
impl Unit<(Rx<U<32>>, In<U<16>>, In<Bit>), (Tx<EthByte>, Out<Bit>, Out<U<16>>)>
    for FrameOut
{
    async fn run(
        &mut self,
        (inp, bytes, go): (Rx<U<32>>, In<U<16>>, In<Bit>),
        (out, running, nwords): (Tx<EthByte>, Out<Bit>, Out<U<16>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let run = self.run.get();
            let have = self.have.get();
            let pos = self.pos.get();
            let want = self.want.get();

            // A request is taken when the unit is idle. `go` is a
            // level and not a pulse, so the register block may hold
            // it until `running` says it was taken.
            let start = go.get() & !run;
            // A frame of no bytes is finished the moment it starts,
            // which is what ends a run rather than the last byte
            // being emitted: `pos` reaching `want` is the one test,
            // so a zero length asks for nothing and stops.
            let more = pos < want;
            let active = run & more;

            // A word is taken when the last one is spent. The bubble
            // that costs is one cycle in five, and it costs nothing
            // on the wire: the transmitter stores a whole frame
            // before it sends any of it, so this side is never what
            // the line waits for.
            let take = active & (have == 0) & Bit::from(inp.peek().is_some());
            let w = inp.recv_if(take).unwrap_or_default();

            let emit = active & (have != 0) & out.ready();
            // The last byte of the frame, which is not the last byte
            // of the last word unless the length divides by four.
            let at_last = Bit::from((pos + 1) == want);

            with!(self <= {
                start ? {
                    run: Bit::One,
                    want: bytes.get(),
                    pos: U::<16>::from(0u8),
                    have: U::<3>::from(0u8),
                },
                take ? {
                    word: w,
                    have: U::<3>::from(4u8),
                },
                emit ? {
                    pos: pos + 1,
                    have: have - 1,
                    word: self.word.get() >> 8u32,
                },
                run & !more ? run: Bit::Zero,
            });

            running.set(run);
            // The words a fetch engine must bring for a frame of
            // `bytes`, the last of them partial. The engine counts
            // words and the register block counts bytes, and a board
            // is only wires between its units, so the one unit that
            // stands between the two is where the division is done.
            // Driven from `bytes` itself rather than from the length
            // latched at the start, since the engine reads it in the
            // same cycle it is told to go.
            nwords.set((bytes.get() + 3) >> 2u32);

            if emit.to_bool() {
                out.send(EthByte {
                    data: self.word.get().slice::<0, 8>(),
                    last: at_last,
                });
            }
        }
    }
}
// end{out}

// begin{in}
/// The receiver's bytes, words to a store engine.
///
/// The bytes of one frame are packed lowest first into 32-bit words.
/// The last word of a frame is usually partial, and it is sent with
/// its real bytes in the low lanes; the store engine strobes away
/// the rest, so nothing past the frame is written.
///
/// The unit also chooses which slot the frame lands in, alternating,
/// and says how long it was. Both are read by the register block:
/// the length when the store finishes, and the slot with it.
///
/// Its ports are named apart from [`FrameOut`]'s on purpose. A
/// testbench reads the trace by port name, so two lowered units in
/// one run that both call a port `out` bind to the same recorded
/// signal, and the one whose width does not match fails in the
/// generated VHDL rather than at the collision (issue 462).
#[derive(Trace, Default)]
pub struct FrameIn {
    /// The frame's length in bytes, as the receiver gave it.
    pub want: Reg<U<16>>,
    /// Bytes taken so far.
    pub pos: Reg<U<16>>,
    /// The word being packed, filled from the top down.
    pub word: Reg<U<32>>,
    /// How many of its bytes are real.
    pub have: Reg<U<3>>,
    /// Whether a frame is being taken.
    pub run: Reg<Bit>,
    /// Which slot the next frame lands in.
    pub slot: Reg<U<1>>,
    /// Whether the frame being taken is being dropped, because both
    /// slots held a frame the driver had not released when it began
    /// (issue 1313): its bytes are taken and nothing is stored.
    pub skip: Reg<Bit>,
    /// The frames dropped so, which the register block reads as
    /// `rx_errors`.
    pub dropped: Reg<U<32>>,
}

#[lower]
impl
    Unit<
        (Rx<EthByte>, In<U<16>>, In<Bit>, In<Bit>),
        (Tx<U<32>>, Out<U<16>>, Out<Bit>, Out<U<1>>, Out<U<32>>),
    > for FrameIn
{
    async fn run(
        &mut self,
        (rx, len, hold, no_slot): (Rx<EthByte>, In<U<16>>, In<Bit>, In<Bit>),
        (words, count, store, which, drops): (
            Tx<U<32>>,
            Out<U<16>>,
            Out<Bit>,
            Out<U<1>>,
            Out<U<32>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let run = self.run.get();
            let have = self.have.get();
            let pos = self.pos.get();
            let want = self.want.get();

            // A frame is there when the receiver offers a byte, and
            // its length is settled by then: the receiver takes a
            // whole frame and checks it before offering any of it.
            let offered = Bit::from(rx.peek().is_some());
            // Not while the store engine is still busy with the frame
            // before. This unit finishes handing words over well before
            // the engine has its write answered, and the register
            // block reads this unit's length and slot at the moment the
            // engine goes idle. Starting the next frame early would
            // change both under the frame still being stored, so the
            // driver would be told the new frame's length and slot for
            // the old frame's bytes. Nothing in a run without a real
            // store engine behind this unit can see that, which is how
            // it was found: by reading the board join, not by a test.
            let start =
                !run & offered & Bit::from(len.get() != 0) & !hold.get();

            // Both slots hold a frame the driver has not released, so
            // the frame starting now is taken and dropped, and counted
            // (issue 1313). Landing it would overwrite one of them.
            let drop_it = no_slot.get();
            let skip = self.skip.get();

            let more = pos < want;
            let full = have == 4;
            // The frame is done and a partial word is still held.
            let tail = !more & (have != 0);
            let emit = run & !skip & Bit::from(full | tail) & words.ready();
            let take = run & more & (skip | Bit::from(!full)) & offered;
            let b = rx.recv_if(take).unwrap_or_default();

            // Packed from the top down, so after four bytes the first
            // one is in the low lane. A word with fewer than four is
            // shifted down by the lanes it never filled, which puts
            // its bytes where the engine's strobe expects them.
            let packed =
                (self.word.get() >> 8u32) | b.data.resize::<32>() << 24u32;
            let held = self.word.get();
            let aligned = mux(
                Bit::from(have == 4),
                held,
                mux(
                    Bit::from(have == 3),
                    held >> 8u32,
                    mux(Bit::from(have == 2), held >> 16u32, held >> 24u32),
                ),
            );

            with!(self <= {
                start ? {
                    run: Bit::One,
                    want: len.get(),
                    pos: U::<16>::from(0u8),
                    have: U::<3>::from(0u8),
                    skip: drop_it,
                },
                // The other slot, by counting round; a dropped frame
                // takes none.
                start & !drop_it ? slot: self.slot.get() + 1,
                start & drop_it ? dropped: self.dropped.get() + 1,
                take & !skip ? {
                    word: packed,
                    have: have + 1,
                },
                take ? pos: pos + 1,
                emit ? have: U::<3>::from(0u8),
                run & Bit::from(!more) & (have == 0) ? run: Bit::Zero,
            });

            count.set(want);
            store.set(run & !skip);
            which.set(self.slot.get());
            drops.set(self.dropped.get());

            if emit.to_bool() {
                words.send(aligned);
            }
        }
    }
}
// end{in}

// begin{len}
/// A frame's bytes, held whole and handed on with their count.
///
/// [`FrameIn`] must know a frame's length before the store engine
/// starts, since the engine is told how many bytes to write when it
/// begins. [`crate::eth::EthRx`] knows it and says so, but on the board
/// the receiver is in the top, on the clock the PHY recovers, and only
/// the bytes cross to the core's clock. The length cannot follow them
/// on a wire of its own, because it would arrive on the wrong clock
/// and out of step with the frame it describes; and it cannot ride
/// inside the stream, because the remote peripheral reads the same
/// stream and would read the length as a byte of its frame.
///
/// So the length is found again after the crossing. This unit takes a
/// frame a byte at a time, holds all of it, and then offers it back
/// with its count on `len`, which is exactly what `EthRx` offers. It
/// is the receiver's output side without the receiver: a unit joined
/// to `FrameIn` in place of an `EthRx` cannot tell the difference.
///
/// It costs a frame of memory and a frame of latency, since the frame
/// was already held whole once, before the crossing. That is the price
/// of the length not crossing with it.
#[derive(Trace, Default)]
pub struct FrameLen {
    /// The frame being taken, or being handed on.
    pub frame: Mem<U<8>, FRAME_MAX>,
    /// Bytes held.
    pub held: Reg<U<11>>,
    /// The next byte to hand on.
    pub pos: Reg<U<11>>,
    /// Taking 0, handing on 1.
    pub phase: Reg<Bit>,
}

#[lower]
impl Unit<Rx<EthByte>, (Tx<EthByte>, Out<U<16>>)> for FrameLen {
    async fn run(
        &mut self,
        inp: Rx<EthByte>,
        (out, len): (Tx<EthByte>, Out<U<16>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let phase = self.phase.get();
            let n = self.held.get();
            let pos = self.pos.get();
            let b = inp.head();

            // Taking: every byte is held, and the last one ends the
            // frame. The last slot is kept rather than overrun, as the
            // receiver does, so a frame longer than the memory keeps
            // overwriting its final byte instead of wrapping.
            let taking = !phase;
            let take = taking & Bit::from(inp.peek().is_some());
            let _ = inp.recv_if(take);
            let full = Bit::from(n == 2047);

            // Handing on: the held bytes in order, the last marked.
            let giving = phase;
            let give = giving & out.ready();
            let at_last = Bit::from((pos + 1) == n);

            with!(self <= {
                take ? frame.at(n): b.data,
                take & !full ? held: n + 1,
                take & b.last ? {
                    phase: Bit::One,
                    pos: U::<11>::from(0u8),
                },
                give ? pos: pos + 1,
                give & at_last ? {
                    phase: Bit::Zero,
                    held: U::<11>::from(0u8),
                },
            });

            // Zero whenever no frame is being handed on, as the
            // receiver's own length is, so that a stale count cannot be
            // read as a current one.
            len.set(mux(giving, n.resize::<16>(), U::<16>::from(0u8)));

            if give.to_bool() {
                out.send(EthByte {
                    data: self.frame.read(pos),
                    last: at_last,
                });
            }
        }
    }
}
// end{len}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{chan, join2, signal, Running};

    /// The word count `FrameOut` asks a fetch engine for, for a frame
    /// of `bytes`.
    fn nwords_for(bytes: u16) -> u16 {
        let (_in_tx, in_rx) = chan::<U<32>, DefaultClock>();
        let (out_tx, _out_rx) = chan::<EthByte, DefaultClock>();
        let (bytes_out, bytes_in) = signal::<U<16>, DefaultClock>();
        let (_go_out, go) = signal::<Bit, DefaultClock>();
        let (running_out, _running) = signal::<Bit, DefaultClock>();
        let (nwords_out, nwords) = signal::<U<16>, DefaultClock>();
        let got: Rc<RefCell<u16>> = Rc::default();
        let log = got.clone();
        let client = async move {
            bytes_out.set(U::from(bytes));
            for _ in 0..4 {
                DefaultClock::rising().await;
            }
            *log.borrow_mut() = nwords.get().raw() as u16;
        };
        let mut unit = FrameOut::default();
        let mut sim = Running::new(join2(
            unit.run((in_rx, bytes_in, go), (out_tx, running_out, nwords_out)),
            client,
        ));
        for _ in 0..10 {
            sim.cycle();
        }
        let n = *got.borrow();
        n
    }

    /// A frame of any length is fetched whole: one word for every four
    /// bytes and one more for any left over (#2049). A word short
    /// leaves `FrameOut` waiting for a word that never comes.
    #[test]
    fn frame_out_asks_for_every_word_of_the_frame() {
        for n in 0..=9u16 {
            assert_eq!(nwords_for(n), n.div_ceil(4), "for {n} bytes");
        }
    }

    /// What `FrameIn` hands the store engine for a frame of `bytes`,
    /// offered whole, with `hold` low throughout.
    fn words_for(bytes: &[u8]) -> Vec<u32> {
        let (in_tx, in_rx) = chan::<EthByte, DefaultClock>();
        let (words_tx, words_rx) = chan::<U<32>, DefaultClock>();
        let (len_out, len) = signal::<U<16>, DefaultClock>();
        let (_hold_out, hold) = signal::<Bit, DefaultClock>();
        let (_no_slot_out, no_slot) = signal::<Bit, DefaultClock>();
        let (count_out, _count) = signal::<U<16>, DefaultClock>();
        let (store_out, _store) = signal::<Bit, DefaultClock>();
        let (which_out, _which) = signal::<U<1>, DefaultClock>();
        let (drops_out, _drops) = signal::<U<32>, DefaultClock>();
        let got: Rc<RefCell<Vec<u32>>> = Rc::default();
        let log = got.clone();
        let frame = bytes.to_vec();
        let client = async move {
            len_out.set(U::from(frame.len() as u16));
            for (i, &b) in frame.iter().enumerate() {
                DefaultClock::rising().await;
                while !in_tx.ready().to_bool() {
                    DefaultClock::rising().await;
                }
                in_tx.send(EthByte {
                    data: U::from(b),
                    last: Bit::from(i + 1 == frame.len()),
                });
            }
        };
        let sink = async move {
            loop {
                DefaultClock::rising().await;
                if let Some(w) = words_rx.recv() {
                    log.borrow_mut().push(w.raw() as u32);
                }
            }
        };
        let mut unit = FrameIn::default();
        let mut sim = Running::new(join2(
            unit.run(
                (in_rx, len, hold, no_slot),
                (words_tx, count_out, store_out, which_out, drops_out),
            ),
            join2(client, sink),
        ));
        for _ in 0..200 {
            sim.cycle();
        }
        let w = got.borrow().clone();
        w
    }

    /// Each tail, of one, two or three bytes, lands in the low lanes of
    /// its word with nothing above it, where the store engine's strobe
    /// expects it (#2049). A frame of 1514 bytes, a full one, ends in
    /// a tail of two.
    #[test]
    fn frame_in_puts_every_tail_in_the_low_lanes() {
        for n in 4..=8usize {
            let bytes: Vec<u8> = (0..n).map(|i| 0x11 * (i as u8 + 1)).collect();
            let want: Vec<u32> = bytes
                .chunks(4)
                .map(|c| {
                    c.iter()
                        .enumerate()
                        .fold(0u32, |w, (i, &b)| w | (b as u32) << (8 * i))
                })
                .collect();
            assert_eq!(words_for(&bytes), want, "a frame of {n} bytes");
        }
    }

    /// A frame offered while the store engine is still busy with the
    /// one before waits for it (#2049): `store` stays low and `count`
    /// keeps the old frame's length until `hold` falls, since the
    /// register block reads both when the engine goes idle.
    #[test]
    fn frame_in_waits_for_the_store_before() {
        let (in_tx, in_rx) = chan::<EthByte, DefaultClock>();
        let (words_tx, words_rx) = chan::<U<32>, DefaultClock>();
        let (len_out, len) = signal::<U<16>, DefaultClock>();
        let (hold_out, hold) = signal::<Bit, DefaultClock>();
        let (_no_slot_out, no_slot) = signal::<Bit, DefaultClock>();
        let (count_out, count) = signal::<U<16>, DefaultClock>();
        let (store_out, store) = signal::<Bit, DefaultClock>();
        let (which_out, _which) = signal::<U<1>, DefaultClock>();
        let (drops_out, _drops) = signal::<U<32>, DefaultClock>();
        // The cycles, counted from the frame's offer, in which `store`
        // was high, and `count` in each.
        let seen: Rc<RefCell<Vec<(u32, u16)>>> = Rc::default();
        let log = seen.clone();
        let client = async move {
            hold_out.set(Bit::One);
            len_out.set(U::from(6u16));
            DefaultClock::rising().await;
            in_tx.send(EthByte {
                data: U::from(0x5au8),
                last: Bit::Zero,
            });
            for t in 0..40u32 {
                DefaultClock::rising().await;
                if t == 20 {
                    hold_out.set(Bit::Zero);
                }
                if store.get().to_bool() {
                    log.borrow_mut().push((t, count.get().raw() as u16));
                }
            }
        };
        let sink = async move {
            loop {
                DefaultClock::rising().await;
                let _ = words_rx.recv();
            }
        };
        let mut unit = FrameIn::default();
        let mut sim = Running::new(join2(
            unit.run(
                (in_rx, len, hold, no_slot),
                (words_tx, count_out, store_out, which_out, drops_out),
            ),
            join2(client, sink),
        ));
        for _ in 0..100 {
            sim.cycle();
        }
        let seen = seen.borrow();
        let first = seen.first().expect("the frame starts once hold falls");
        assert!(first.0 > 20, "started under hold, at {}", first.0);
        assert_eq!(first.1, 6, "the new frame's length, once started");
    }
}
