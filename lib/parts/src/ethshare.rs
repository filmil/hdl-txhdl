// SPDX-License-Identifier: Apache-2.0
//! One Ethernet wire, two users, split by EtherType.
//!
//! The board has one Ethernet port and two things that want it: the
//! remote peripheral of issue 297, whose transport is one frame per
//! transaction, and the port a Zephyr driver talks to through
//! [`crate::ethslots::EthSlots`]. Neither can have the wire to itself.
//!
//! Nothing new has to be invented to share it, because Ethernet
//! already carries several protocols on one wire and says which is
//! which: bytes twelve and thirteen of every frame are its EtherType.
//! The remote peripheral sends `0x88b5`, which IEEE leaves for local
//! use, and Zephyr's stack sends only the standard types, so the two
//! cannot be confused.
//!
//! # Receiving
//!
//! A frame's user is not known until its fourteenth byte, and the
//! thirteen before it belong to that user too. So the unit holds the
//! first fourteen bytes, reads the type from the last two of them,
//! and then sends the whole frame, held bytes first, to one user:
//! frames of type `TYPE` to the first, every other frame to the
//! second.
//!
//! A frame that ends before its type is known goes to the second
//! user rather than being dropped. The receiver checks every frame
//! and pads a short one, so this should not arrive; but a unit that
//! waits for a fourteenth byte that never comes would hold the wire
//! for ever, and that is worse than delivering a frame nobody wants.
//!
//! # Sending
//!
//! A frame cannot be interleaved with another, so the two senders are
//! merged a frame at a time. When both offer, they take turns: the
//! one that did not send last goes first. Neither can then starve the
//! other, whichever one sends more.
use txhdl::comp::{mux, Clock, DefaultClock, Mem, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

use crate::eth::EthByte;

/// Bytes of a frame before its EtherType is known, the type included.
pub const HEADER: usize = 14;

// begin{state}
/// The shared wire. Frames of type `TYPE` go to the first user.
#[derive(Trace, Default)]
pub struct EthShare<const TYPE: usize> {
    /// The first fourteen bytes of the frame arriving.
    pub hdr: Mem<U<8>, 16>,
    /// How many of them are held.
    pub held: Reg<U<4>>,
    /// The next held byte to hand on.
    pub at: Reg<U<4>>,
    /// Receiving: 0 holding the header, 1 handing it on, 2 passing
    /// the rest of the frame through.
    pub rph: Reg<U<2>>,
    /// Whether the frame arriving goes to the first user.
    pub to_a: Reg<Bit>,
    /// Byte twelve, the high half of the EtherType.
    pub ty_hi: Reg<U<8>>,
    /// The frame ended while its header was being held, so the last
    /// held byte is its last.
    pub short: Reg<Bit>,
    /// A frame is going out.
    pub tbusy: Reg<Bit>,
    /// Which sender it is from: low the first user, high the second.
    pub tsel: Reg<Bit>,
    /// Who goes first when both offer: low the first user.
    pub turn: Reg<Bit>,
}
// end{state}

// begin{run}
#[lower]
impl<const TYPE: usize>
    Unit<
        (Rx<EthByte>, Rx<EthByte>, Rx<EthByte>),
        (Tx<EthByte>, Tx<EthByte>, Tx<EthByte>),
    > for EthShare<TYPE>
{
    async fn run(
        &mut self,
        (wire_in, a_in, b_in): (Rx<EthByte>, Rx<EthByte>, Rx<EthByte>),
        (wire_out, a_out, b_out): (Tx<EthByte>, Tx<EthByte>, Tx<EthByte>),
    ) {
        loop {
            DefaultClock::rising().await;

            // Receiving.
            let rph = self.rph.get();
            let held = self.held.get();
            let at = self.at.get();
            let to_a = self.to_a.get();
            let short = self.short.get();
            let w = wire_in.head();
            let offered = Bit::from(wire_in.peek().is_some());

            let holding = Bit::from(rph == 0);
            let replaying = Bit::from(rph == 1);
            let passing = Bit::from(rph == 2);

            // Holding does not wait for either user: the header is
            // kept here until its user is known.
            let take_hdr = holding & offered;
            let is_12 = Bit::from(held == 12);
            let is_13 = Bit::from(held == 13);
            // The type is byte twelve over the byte arriving now.
            let ty = self.ty_hi.get().concat::<8, 16>(w.data);
            let match_a = Bit::from(ty == U::<16>::from(TYPE as u32));
            // The header is done at its fourteenth byte, or earlier
            // if the frame ends first.
            let leave = take_hdr & (is_13 | w.last);

            let dest_ready = (to_a & a_out.ready()) | (!to_a & b_out.ready());
            let hbyte = self.hdr.read(at);
            let at_end = Bit::from((at + 1) == held);
            let replay_go = replaying & dest_ready;
            let pass_go = passing & offered & dest_ready;
            let _ = wire_in.recv_if(take_hdr | pass_go);

            // The byte handed on, from the held header or the wire.
            let r_data = mux(replaying, hbyte, w.data);
            // A held byte is the frame's last only when the frame
            // ended while it was being held.
            let r_last = (replaying & short & at_end) | (!replaying & w.last);
            let handed = replay_go | pass_go;

            // Sending.
            let tbusy = self.tbusy.get();
            let from_b = self.tsel.get();
            let turn = self.turn.get();
            let ah = a_in.head();
            let bh = b_in.head();
            let a_offers = Bit::from(a_in.peek().is_some());
            let b_offers = Bit::from(b_in.peek().is_some());
            // The first user goes first unless it is the second's
            // turn and the second has something to send.
            let pick_a = a_offers & (!turn | !b_offers);
            let start = !tbusy & (a_offers | b_offers);
            let src_offers = (from_b & b_offers) | (!from_b & a_offers);
            let fwd = tbusy & src_offers & wire_out.ready();
            let _ = a_in.recv_if(fwd & !from_b);
            let _ = b_in.recv_if(fwd & from_b);
            let t_data = mux(from_b, bh.data, ah.data);
            let t_last = (from_b & bh.last) | (!from_b & ah.last);

            with!(self <= {
                take_hdr ? {
                    hdr.at(held): w.data,
                    held: held + 1,
                },
                take_hdr & is_12 ? ty_hi: w.data,
                leave ? {
                    to_a: is_13 & match_a,
                    rph: U::<2>::from(1u8),
                    at: U::<4>::from(0u8),
                    short: w.last,
                },
                replay_go ? at: at + 1,
                replay_go & at_end & short ? {
                    rph: U::<2>::from(0u8),
                    held: U::<4>::from(0u8),
                },
                replay_go & at_end & !short ? rph: U::<2>::from(2u8),
                pass_go & w.last ? {
                    rph: U::<2>::from(0u8),
                    held: U::<4>::from(0u8),
                },
                start ? {
                    tbusy: Bit::One,
                    tsel: !pick_a,
                },
                // The next turn is the other sender's.
                fwd & t_last ? {
                    tbusy: Bit::Zero,
                    turn: !from_b,
                },
            });

            if (handed & to_a).to_bool() {
                a_out.send(EthByte {
                    data: r_data,
                    last: r_last,
                });
            }
            if (handed & !to_a).to_bool() {
                b_out.send(EthByte {
                    data: r_data,
                    last: r_last,
                });
            }
            if fwd.to_bool() {
                wire_out.send(EthByte {
                    data: t_data,
                    last: t_last,
                });
            }
        }
    }
}
// end{run}
