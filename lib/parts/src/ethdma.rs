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
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

use crate::eth::EthByte;

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
impl Unit<(Rx<U<32>>, In<U<16>>, In<Bit>), (Tx<EthByte>, Out<Bit>)>
    for FrameOut
{
    async fn run(
        &mut self,
        (inp, bytes, go): (Rx<U<32>>, In<U<16>>, In<Bit>),
        (out, running): (Tx<EthByte>, Out<Bit>),
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
}

#[lower]
impl
    Unit<(Rx<EthByte>, In<U<16>>), (Tx<U<32>>, Out<U<16>>, Out<Bit>, Out<U<1>>)>
    for FrameIn
{
    async fn run(
        &mut self,
        (rx, len): (Rx<EthByte>, In<U<16>>),
        (words, count, store, which): (
            Tx<U<32>>,
            Out<U<16>>,
            Out<Bit>,
            Out<U<1>>,
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
            let start = !run & offered & Bit::from(len.get() != 0);

            let more = pos < want;
            let full = have == 4;
            // The frame is done and a partial word is still held.
            let tail = !more & (have != 0);
            let emit = run & Bit::from(full | tail) & words.ready();
            let take = run & more & Bit::from(!full) & offered;
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
                    // Alternating, written as a toggle rather than
                    // as `+ 1`: a one-bit register incremented lowers
                    // to `slot + '1'` in VHDL, which is a character
                    // and not an unsigned, and nvc refuses it
                    // (issue 461).
                    slot: !self.slot.get(),
                },
                take ? {
                    word: packed,
                    have: have + 1,
                    pos: pos + 1,
                },
                emit ? have: U::<3>::from(0u8),
                run & Bit::from(!more) & (have == 0) ? run: Bit::Zero,
            });

            count.set(want);
            store.set(run);
            which.set(self.slot.get());

            if emit.to_bool() {
                words.send(aligned);
            }
        }
    }
}
// end{in}
