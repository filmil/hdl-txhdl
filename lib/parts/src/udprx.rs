// SPDX-License-Identifier: Apache-2.0
//! A UDP/IPv4 receive filter, after the MAC's receiver (#1840).
//!
//! [`crate::eth::EthRx`] offers only frames whose check sequence held,
//! without it, from the destination address on, with the last byte
//! marked; a short frame still has its padding up to sixty bytes.
//! [`UdpRx`] takes those frames and passes on the payload of each UDP
//! datagram addressed to the MAC address, IPv4 address and port its
//! configuration names, and nothing of any other frame.
//!
//! # Checks
//!
//! Every header byte is checked as it goes by, and none is held. The
//! checks are done by the forty-second byte, where the payload starts,
//! and nothing is sent before it, so a frame that fails one gives no
//! byte at all. By the frame's byte offset:
//!
//! * 0 to 5: the destination address is the configured one; or, when
//!   the configured IPv4 address is a multicast group (224.0.0.0/4),
//!   the group's: `01:00:5e` and the group's low 23 bits. Broadcast is
//!   refused.
//! * 12 and 13: the type is IPv4, `0x0800`; a VLAN tag is refused.
//! * 14: `0x45`, version 4 and a header of five words: options are
//!   refused.
//! * 16 and 17: the total length, which must be at least the UDP
//!   length and the twenty bytes of the IPv4 header.
//! * 20 and 21: more-fragments clear and the fragment offset zero;
//!   don't-fragment is ignored.
//! * 23: protocol 17, UDP.
//! * 14 to 33: the header checksum, whose ones' complement sum over the
//!   header is all ones, summed a pair of bytes at a time.
//! * 30 to 33: the destination address is the configured one.
//! * 36 and 37: the destination port is the configured one.
//! * 38 and 39: the UDP length is at least nine: an empty datagram has
//!   no byte to mark last, so it is refused.
//!
//! The source address and port, the time to live and the UDP checksum
//! are not checked. The UDP checksum covers the payload, so checking it
//! would mean holding the whole datagram, and the MAC's CRC-32 has
//! already checked every byte of the frame.
//!
//! # The payload
//!
//! The payload is the UDP length less eight bytes. Its last byte goes
//! out marked last; if the frame ends first, its last byte is marked
//! instead, and the datagram is cut short there. What follows the
//! payload, padding included, is taken and dropped. The count rises
//! with each datagram's last byte, and wraps.
//!
//! A payload byte is taken from the receiver only when the output is
//! ready; a header byte, or a byte being dropped, is taken whenever it
//! is offered.
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::Value as ValueDerive;
use txhdl::{lower, select, with, Trace};

use crate::eth::EthByte;

/// The header's bytes: Ethernet's fourteen, IPv4's twenty and UDP's
/// eight. The payload starts at this offset.
pub const HEADER: u32 = 42;

/// What a [`UdpRx`] takes: its MAC address, its IPv4 address or the
/// multicast group it listens to, and its UDP port.
#[derive(ValueDerive, Clone, Copy, Default, PartialEq, Debug)]
pub struct UdpCfg {
    /// The MAC address, its first byte on the wire in bits 47 to 40.
    /// Not used when `ip` is a multicast group.
    pub mac: U<48>,
    /// The IPv4 address, its first byte in bits 31 to 24.
    pub ip: U<32>,
    /// The UDP port.
    pub port: U<16>,
}

// begin{state}
/// The filter.
#[derive(Trace, Default)]
pub struct UdpRx {
    /// The offset in the frame of the next header byte.
    pub pos: Reg<U<6>>,
    /// 0 checking the header, 1 sending the payload, 2 dropping the
    /// rest of the frame.
    pub phase: Reg<U<2>>,
    /// A check on this frame's header has failed.
    pub bad: Reg<Bit>,
    /// The byte before this one, the high half of a 16-bit field.
    pub hi: Reg<U<8>>,
    /// The IPv4 header's ones' complement sum so far.
    pub sum: Reg<U<16>>,
    /// The IPv4 total length.
    pub iplen: Reg<U<16>>,
    /// The UDP length.
    pub ulen: Reg<U<16>>,
    /// Payload bytes still to send.
    pub left: Reg<U<16>>,
    /// Datagrams passed on.
    pub passed: Reg<U<32>>,
}
// end{state}

// begin{run}
#[lower]
impl Unit<(Rx<EthByte>, In<UdpCfg>), (Tx<EthByte>, Out<U<32>>)> for UdpRx {
    async fn run(
        &mut self,
        (rx, cfg): (Rx<EthByte>, In<UdpCfg>),
        (tx, count): (Tx<EthByte>, Out<U<32>>),
    ) {
        loop {
            DefaultClock::rising().await;
            count.set(self.passed.get());
            let c = cfg.get();
            let pos = self.pos.get();
            let phase = self.phase.get();
            let bad = self.bad.get();
            let hi = self.hi.get();
            let w = rx.head();
            let d = w.data;
            let offered = Bit::from(rx.peek().is_some());

            let checking = Bit::from(phase == 0);
            let sending = Bit::from(phase == 1);
            let dropping = Bit::from(phase == 2);

            // The destination address a frame must carry: the group's
            // for a multicast group, else the configured one.
            let mcast = Bit::from(c.ip.slice::<28, 4>() == 0xe);
            let gmac = U::<48>::from(0x0100_5e00_0000u64)
                | c.ip.slice::<0, 23>().resize::<48>();
            let mac = mux(mcast, gmac, c.mac);

            // A byte that must equal a known value at its offset.
            let fixed = Bit::from(pos < 6)
                | Bit::from(pos == 12)
                | Bit::from(pos == 13)
                | Bit::from(pos == 14)
                | Bit::from(pos == 21)
                | Bit::from(pos == 23)
                | (Bit::from(pos >= 30) & Bit::from(pos < 34));
            let want = select!(pos.raw() => {
                0 => mac.slice::<40, 8>(),
                1 => mac.slice::<32, 8>(),
                2 => mac.slice::<24, 8>(),
                3 => mac.slice::<16, 8>(),
                4 => mac.slice::<8, 8>(),
                5 => mac.slice::<0, 8>(),
                12 => U::<8>::from(0x08u8),
                14 => U::<8>::from(0x45u8),
                23 => U::<8>::from(17u8),
                30 => c.ip.slice::<24, 8>(),
                31 => c.ip.slice::<16, 8>(),
                32 => c.ip.slice::<8, 8>(),
                33 => c.ip.slice::<0, 8>(),
                _ => U::<8>::from(0u8),
            });

            // The 16-bit field that ends at this byte.
            let word = hi.concat::<8, 16>(d);
            // The checksum's sum with this byte's pair added, the carry
            // out of the top folded back in: at the odd offsets 15 to
            // 33, the IPv4 header's second byte of each pair.
            let in_ip = Bit::from(pos >= 14) & Bit::from(pos < 34);
            let odd = pos.bit(0);
            let wide = self.sum.get().resize::<17>() + word.resize::<17>();
            let folded =
                wide.slice::<0, 16>() + wide.slice::<16, 1>().resize::<16>();
            // The UDP length, at offset 39, against what the IPv4
            // header said, both taken wide enough not to wrap.
            let udplen = word;
            let iplen = self.iplen.get();
            let len_bad = Bit::from(udplen < 9)
                | Bit::from(
                    iplen.resize::<17>()
                        < udplen.resize::<17>() + U::<17>::from(20u32),
                );

            let fail = (fixed & Bit::from(d != want))
                | (Bit::from(pos == 20) & Bit::from(d.slice::<0, 6>() != 0))
                | (Bit::from(pos == 33) & Bit::from(folded != 0xffff))
                | (Bit::from(pos == 37) & Bit::from(word != c.port))
                | (Bit::from(pos == 39) & len_bad);

            // Checking takes every byte offered.
            let take_hdr = checking & offered;
            let bad_now = bad | fail;
            let hdr_done = take_hdr & Bit::from(pos == HEADER - 1);
            // The header ended with no check failed, and the frame goes
            // on: the payload follows.
            let to_send = hdr_done & !bad_now & !w.last;
            let to_drop = hdr_done & (bad_now & !w.last);
            // A frame that ends in its header gives nothing.
            let hdr_end = take_hdr & w.last;

            // Sending waits for the output.
            let left = self.left.get();
            let send = sending & offered & tx.ready();
            let at_len = Bit::from(left == 1);
            let out_last = w.last | at_len;
            let send_end = send & out_last;
            let drop_go = dropping & offered;

            let _ = rx.recv_if(take_hdr | send | drop_go);

            with!(self <= {
                take_hdr ? {
                    pos: pos + 1,
                    hi: d,
                    bad: bad_now,
                },
                take_hdr & Bit::from(pos == 17) ? iplen: word,
                take_hdr & Bit::from(pos == 39) ? ulen: word,
                take_hdr & Bit::from(pos == 14) ? sum: U::<16>::from(0u16),
                take_hdr & in_ip & odd ? sum: folded,
                to_send ? {
                    phase: U::<2>::from(1u8),
                    left: self.ulen.get() - 8,
                },
                to_drop ? phase: U::<2>::from(2u8),
                hdr_end ? {
                    pos: U::<6>::from(0u8),
                    bad: Bit::Zero,
                },
                send ? left: left - 1,
                send_end ? passed: self.passed.get() + 1,
                send_end & w.last ? {
                    phase: U::<2>::from(0u8),
                    pos: U::<6>::from(0u8),
                    bad: Bit::Zero,
                },
                send_end & !w.last ? phase: U::<2>::from(2u8),
                drop_go & w.last ? {
                    phase: U::<2>::from(0u8),
                    pos: U::<6>::from(0u8),
                    bad: Bit::Zero,
                },
            });

            if send.to_bool() {
                tx.send(EthByte {
                    data: d,
                    last: out_last,
                });
            }
        }
    }
}
// end{run}

