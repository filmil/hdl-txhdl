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

#[cfg(test)]
mod tests {
    use super::*;
    use txhdl::comp::{chan, signal, Running};

    const MAC: [u8; 6] = [0x02, 0x00, 0x5e, 0x10, 0x20, 0x30];
    const IP: [u8; 4] = [10, 0, 0, 2];
    const PORT: u16 = 5000;

    /// The ones' complement checksum of RFC 1071, over pairs of bytes.
    fn checksum(b: &[u8]) -> u16 {
        let mut s: u32 = 0;
        for p in b.chunks(2) {
            let hi = p[0] as u32;
            let lo = *p.get(1).unwrap_or(&0) as u32;
            s += (hi << 8) | lo;
        }
        while s > 0xffff {
            s = (s & 0xffff) + (s >> 16);
        }
        !(s as u16)
    }

    /// A datagram's fields, each of which a test may change.
    #[derive(Clone)]
    struct Dgram {
        dst: [u8; 6],
        ty: u16,
        vihl: u8,
        /// The IPv4 total length; `None` is the right one.
        total: Option<u16>,
        frag: u16,
        proto: u8,
        dst_ip: [u8; 4],
        dport: u16,
        /// The UDP length; `None` is the right one.
        ulen: Option<u16>,
        payload: Vec<u8>,
        /// Spoil the header checksum.
        bad_sum: bool,
        /// Bytes taken off the frame's end, as a cut-short frame.
        cut: usize,
    }

    impl Dgram {
        fn new(payload: &[u8]) -> Self {
            Dgram {
                dst: MAC,
                ty: 0x0800,
                vihl: 0x45,
                total: None,
                frag: 0x4000,
                proto: 17,
                dst_ip: IP,
                dport: PORT,
                ulen: None,
                payload: payload.to_vec(),
                bad_sum: false,
                cut: 0,
            }
        }

        /// The frame as `EthRx` offers it: padded to sixty bytes.
        fn frame(&self) -> Vec<u8> {
            let ulen = self.ulen.unwrap_or((8 + self.payload.len()) as u16);
            let total = self.total.unwrap_or((28 + self.payload.len()) as u16);
            let mut f = self.dst.to_vec();
            f.extend([0x02, 0, 0, 0, 0, 0x99]);
            f.extend(self.ty.to_be_bytes());
            let mut ip = vec![self.vihl, 0];
            ip.extend(total.to_be_bytes());
            ip.extend([0x12, 0x34]);
            ip.extend(self.frag.to_be_bytes());
            ip.extend([64, self.proto, 0, 0]);
            ip.extend([10, 0, 0, 1]);
            ip.extend(self.dst_ip);
            let mut s = checksum(&ip);
            if self.bad_sum {
                s ^= 0x0100;
            }
            ip[10..12].copy_from_slice(&s.to_be_bytes());
            f.extend(ip);
            f.extend(4321u16.to_be_bytes());
            f.extend(self.dport.to_be_bytes());
            f.extend(ulen.to_be_bytes());
            f.extend([0, 0]);
            f.extend(&self.payload);
            f.truncate(f.len() - self.cut.min(f.len() - 1));
            if f.len() < 60 {
                f.resize(60, 0);
            }
            f
        }
    }

    fn cfg_for(mac: [u8; 6], ip: [u8; 4]) -> UdpCfg {
        let mut m = 0u64;
        for b in mac {
            m = (m << 8) | b as u64;
        }
        UdpCfg {
            mac: U::from(m),
            ip: U::from(u32::from_be_bytes(ip)),
            port: U::from(PORT),
        }
    }

    /// What the filter should pass of `f`, by the rules in the module's
    /// comment, written over again from them.
    fn reference(f: &[u8], cfg: &UdpCfg) -> Option<Vec<u8>> {
        if f.len() <= HEADER as usize {
            return None;
        }
        let ip = (cfg.ip.raw() as u32).to_be_bytes();
        let mac: [u8; 6] = if ip[0] >> 4 == 0xe {
            [0x01, 0x00, 0x5e, ip[1] & 0x7f, ip[2], ip[3]]
        } else {
            let m = (cfg.mac.raw() as u64).to_be_bytes();
            m[2..8].try_into().unwrap()
        };
        let be = |i: usize| u16::from_be_bytes([f[i], f[i + 1]]);
        let total = be(16);
        let ulen = be(38);
        let ok = f[0..6] == mac
            && be(12) == 0x0800
            && f[14] == 0x45
            && f[20] & 0x3f == 0
            && f[21] == 0
            && f[23] == 17
            && checksum(&f[14..34]) == 0
            && f[30..34] == ip
            && be(36) == cfg.port.raw() as u16
            && ulen >= 9
            && total as u32 >= ulen as u32 + 20;
        if !ok {
            return None;
        }
        let end = (HEADER as usize + ulen as usize - 8).min(f.len());
        Some(f[HEADER as usize..end].to_vec())
    }

    /// Run the frames through a filter set to `cfg`, its output taken
    /// on the cycles `take` says; the datagrams out, and the count.
    fn run(
        frames: &[Vec<u8>],
        cfg: UdpCfg,
        take: &mut dyn FnMut(usize) -> bool,
    ) -> (Vec<Vec<u8>>, u32) {
        let (in_tx, in_rx) = chan::<EthByte, DefaultClock>();
        let (out_tx, out_rx) = chan::<EthByte, DefaultClock>();
        let (cfg_o, cfg_i) = signal::<UdpCfg, DefaultClock>();
        let (count_o, count_i) = signal::<U<32>, DefaultClock>();
        cfg_o.set(cfg);
        let mut part = UdpRx::default();
        let mut sim = Running::new(part.run((in_rx, cfg_i), (out_tx, count_o)));
        let mut queue: Vec<EthByte> = Vec::new();
        for f in frames {
            for (i, b) in f.iter().enumerate() {
                queue.push(EthByte {
                    data: U::from(*b),
                    last: Bit::from_bool(i + 1 == f.len()),
                });
            }
        }
        let mut got: Vec<Vec<u8>> = vec![Vec::new()];
        let mut quiet = 0;
        let mut t = 0;
        while quiet < 200 {
            if !queue.is_empty() && in_tx.ready().to_bool() {
                in_tx.send(queue.remove(0));
                quiet = 0;
            }
            let taken = if take(t) { out_rx.recv() } else { None };
            if let Some(b) = taken {
                got.last_mut().unwrap().push(b.data.raw() as u8);
                if b.last.to_bool() {
                    got.push(Vec::new());
                }
                quiet = 0;
            }
            sim.cycle();
            quiet += 1;
            t += 1;
        }
        assert!(queue.is_empty(), "the filter stopped taking bytes");
        assert!(got.pop().unwrap().is_empty(), "a datagram had no last byte");
        (got, count_i.get().raw() as u32)
    }

    fn always(_: usize) -> bool {
        true
    }

    fn pass(d: &Dgram) -> (Vec<Vec<u8>>, u32) {
        run(&[d.frame()], cfg_for(MAC, IP), &mut always)
    }

    /// The header checksum the helper writes agrees with a header whose
    /// sum is published, so the helper and the part cannot share a
    /// mistake in it: the IPv4 header of the example in Wikipedia's
    /// "IPv4 header checksum", 45 00 00 73 ... c0 a8 00 c7, sums to
    /// b861.
    #[test]
    fn the_helpers_checksum_matches_a_published_header() {
        let mut h = [
            0x45, 0x00, 0x00, 0x73, 0x00, 0x00, 0x40, 0x00, 0x40, 0x11, 0x00,
            0x00, 0xc0, 0xa8, 0x00, 0x01, 0xc0, 0xa8, 0x00, 0xc7,
        ];
        assert_eq!(checksum(&h), 0xb861);
        h[10..12].copy_from_slice(&0xb861u16.to_be_bytes());
        assert_eq!(checksum(&h), 0, "a header with its sum sums to zero");
    }

    #[test]
    fn a_datagram_to_the_address_and_port_passes() {
        let d = Dgram::new(b"hello, udp");
        assert_eq!(pass(&d), (vec![b"hello, udp".to_vec()], 1));
    }

    #[test]
    fn a_one_byte_datagram_passes() {
        assert_eq!(pass(&Dgram::new(b"x")), (vec![b"x".to_vec()], 1));
    }

    #[test]
    fn a_short_frames_padding_is_dropped() {
        // Five bytes of payload in a frame padded to sixty.
        let d = Dgram::new(b"abcde");
        assert_eq!(d.frame().len(), 60);
        assert_eq!(pass(&d), (vec![b"abcde".to_vec()], 1));
    }

    #[test]
    fn a_datagram_cut_short_by_the_frame_ends_with_it() {
        let mut d = Dgram::new(&[7u8; 40]);
        d.cut = 15;
        assert_eq!(pass(&d), (vec![vec![7u8; 25]], 1));
    }

    #[test]
    fn a_multicast_group_takes_its_own_mac_address() {
        let group = [239, 129, 2, 3];
        let gmac = [0x01, 0x00, 0x5e, 0x01, 0x02, 0x03];
        let mut d = Dgram::new(b"to the group");
        d.dst_ip = group;
        d.dst = gmac;
        let cfg = cfg_for(MAC, group);
        let got = run(&[d.frame()], cfg, &mut always);
        assert_eq!(got, (vec![b"to the group".to_vec()], 1));
        // The configured unicast address is not the group's.
        d.dst = MAC;
        assert_eq!(run(&[d.frame()], cfg, &mut always), (vec![], 0));
    }

    /// Each check refuses the whole frame: no byte, no count.
    #[test]
    fn every_check_refuses_the_frame_whole() {
        type Spoil = fn(&mut Dgram);
        let cases: [(&str, Spoil); 17] = [
            ("another MAC", |d| d.dst[5] ^= 1),
            ("broadcast", |d| d.dst = [0xff; 6]),
            ("ARP", |d| d.ty = 0x0806),
            ("a VLAN tag", |d| d.ty = 0x8100),
            ("IPv6's version", |d| d.vihl = 0x65),
            ("options", |d| d.vihl = 0x46),
            ("more fragments", |d| d.frag = 0x2000),
            ("an offset", |d| d.frag = 0x0001),
            ("a high offset", |d| d.frag = 0x1000),
            ("TCP", |d| d.proto = 6),
            ("a bad sum", |d| d.bad_sum = true),
            ("another IP", |d| d.dst_ip[3] ^= 1),
            ("another port", |d| d.dport ^= 0x100),
            ("an empty datagram", |d| d.ulen = Some(8)),
            ("a UDP length under 8", |d| d.ulen = Some(3)),
            ("more than the IP packet", |d| d.ulen = Some(40)),
            ("a short total", |d| d.total = Some(20)),
        ];
        for (what, spoil) in cases {
            let mut d = Dgram::new(b"never seen");
            spoil(&mut d);
            assert_eq!(pass(&d), (vec![], 0), "{what}");
            assert_eq!(
                reference(&d.frame(), &cfg_for(MAC, IP)),
                None,
                "{what}"
            );
        }
    }

    #[test]
    fn frames_back_to_back_each_pass_or_not_on_their_own() {
        let good = Dgram::new(b"one");
        let mut bad = Dgram::new(b"two");
        bad.dport = 1;
        let three = Dgram::new(&[3u8; 100]);
        let frames = [good.frame(), bad.frame(), three.frame()];
        let got = run(&frames, cfg_for(MAC, IP), &mut always);
        assert_eq!(got, (vec![b"one".to_vec(), vec![3u8; 100]], 2));
    }

    /// A tiny generator, seeded, for the stalls and the random frames.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// Frames of every kind, most valid but for one field, against the
    /// reference, with the output taken on two cycles in three.
    #[test]
    fn random_frames_agree_with_the_reference() {
        let mut r = Rng(0x9e37_79b9_7f4a_7c15);
        let cfg = cfg_for(MAC, IP);
        let mut frames = vec![];
        for _ in 0..300 {
            let n = r.below(80) as usize;
            let payload: Vec<u8> = (0..n).map(|_| r.next() as u8).collect();
            let mut d = Dgram::new(&payload);
            match r.below(14) {
                0 => d.dst[r.below(6) as usize] ^= 1 << r.below(8),
                1 => d.ty ^= 1 << r.below(16),
                2 => d.vihl ^= 1 << r.below(8),
                3 => d.frag ^= 1 << r.below(16),
                4 => d.proto ^= 1 << r.below(8),
                5 => d.bad_sum = true,
                6 => d.dst_ip[r.below(4) as usize] ^= 1 << r.below(8),
                7 => d.dport ^= 1 << r.below(16),
                8 => d.ulen = Some(r.below(120) as u16),
                9 => d.total = Some(r.below(160) as u16),
                10 => d.cut = r.below(20) as usize,
                _ => {}
            }
            frames.push(d.frame());
        }
        let want: Vec<Vec<u8>> =
            frames.iter().filter_map(|f| reference(f, &cfg)).collect();
        assert!(want.len() > 60, "enough frames pass: {}", want.len());
        let mut stall = Rng(7);
        let (got, count) = run(&frames, cfg, &mut |_| stall.below(3) != 0);
        assert_eq!(count as usize, want.len());
        assert_eq!(got, want);
    }

    #[test]
    fn it_lowers() {
        let net = UdpRx::lowered("udp_rx");
        assert!(net.verilog().contains("module udp_rx"));
    }
}
