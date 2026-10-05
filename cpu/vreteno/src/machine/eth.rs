// SPDX-License-Identifier: Apache-2.0
//! The Ethernet port as the machine model has it (issue 1203): the
//! slot registers of `txhdl_parts::ethslots` at LiteX's offsets, the
//! engines as copies to and from the DDR3, and a peer on the far end of
//! the cable that answers ARP and ping, so that Linux's own
//! `litex_liteeth` can be shown to send and receive.
//!
//! The registers are read from the hardware's own map, by name, so the
//! model and the part cannot disagree about an offset. Two slots a
//! direction, 2048 bytes each, receive at the buffers' base and
//! transmit 4096 bytes above it, as the part has them.
//!
//! A frame is sent the moment `tx_start` is written, which the part
//! takes a burst at a time; the model's driver sees `tx_ready` high
//! again at once. A frame arrives when the receive side has nothing
//! pending, into the slot after the last, as the part's store engine
//! alternates them.
use std::collections::VecDeque;
use txhdl_parts::ethslots::regs;

/// A slot's bytes.
pub const SLOT: u32 = 2048;
/// Transmit slots start this far above the receive slots.
pub const TX_REGION: u32 = 0x1000;

/// The peer: a station on the far end of the cable.
pub const PEER_MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x02];
pub const PEER_IP: [u8; 4] = [10, 0, 0, 2];

/// The port's registers, its traffic, and the frames waiting to arrive.
#[derive(Debug, Default)]
pub struct Eth {
    pub rx_slot: u32,
    pub rx_length: u32,
    pub rx_pending: bool,
    pub rx_enable: bool,
    pub tx_slot: u32,
    pub tx_length: u32,
    pub tx_pending: bool,
    pub tx_enable: bool,
    /// The slot the next arrival goes into.
    pub next_rx: u32,
    /// Every frame sent, in order.
    pub sent: Vec<Vec<u8>>,
    /// Frames on the wire towards the port, not yet in a slot.
    pub inbox: VecDeque<Vec<u8>>,
    /// Frames delivered into a slot.
    pub received: usize,
    /// Whether the peer answers what is sent.
    pub peer: bool,
}

/// What a write did that the bus has to finish: a frame to send from
/// the transmit slot.
pub enum Effect {
    None,
    Send { slot: u32, len: u32 },
}

impl Eth {
    /// The port's interrupt line.
    pub fn irq(&self) -> bool {
        self.rx_pending && self.rx_enable
    }

    /// A word read at byte offset `off`.
    pub fn load(&self, off: u32) -> u32 {
        let b = |v: bool| v as u32;
        match off {
            regs::rx_slot => self.rx_slot,
            regs::rx_length => self.rx_length,
            regs::rx_ev_status | regs::rx_ev_pending => b(self.rx_pending),
            regs::rx_ev_enable => b(self.rx_enable),
            regs::tx_ready => 1,
            regs::tx_ev_pending => b(self.tx_pending),
            regs::tx_ev_enable => b(self.tx_enable),
            _ => 0,
        }
    }

    /// A word written at byte offset `off`.
    pub fn store(&mut self, off: u32, v: u32) -> Effect {
        match off {
            regs::rx_ev_pending if v & 1 != 0 => self.rx_pending = false,
            regs::rx_ev_enable => self.rx_enable = v & 1 != 0,
            regs::tx_slot => self.tx_slot = v & 1,
            regs::tx_length => self.tx_length = v & 0xffff,
            regs::tx_ev_pending if v & 1 != 0 => self.tx_pending = false,
            regs::tx_ev_enable => self.tx_enable = v & 1 != 0,
            regs::tx_start if v & 1 != 0 => {
                return Effect::Send {
                    slot: self.tx_slot,
                    len: self.tx_length,
                }
            }
            _ => {}
        }
        Effect::None
    }

    /// A frame sent: kept, and answered by the peer if it answers.
    pub fn sent(&mut self, frame: Vec<u8>) {
        if self.peer {
            if let Some(reply) = answer(&frame) {
                self.inbox.push_back(reply);
            }
        }
        self.sent.push(frame);
    }

    /// The next frame to put in a slot, and the slot, when the receive
    /// side has room: nothing pending, as the part delivers.
    pub fn arrival(&mut self) -> Option<(u32, Vec<u8>)> {
        if self.rx_pending {
            return None;
        }
        let f = self.inbox.pop_front()?;
        let slot = self.next_rx;
        self.next_rx ^= 1;
        self.rx_slot = slot;
        self.rx_length = f.len() as u32;
        self.rx_pending = true;
        self.received += 1;
        Some((slot, f))
    }
}

/// The ones' complement sum of 16-bit words, as IP and ICMP check.
pub fn checksum(b: &[u8]) -> u16 {
    let mut s: u32 = 0;
    for c in b.chunks(2) {
        s += (c[0] as u32) << 8 | *c.get(1).unwrap_or(&0) as u32;
    }
    while s >> 16 != 0 {
        s = (s & 0xffff) + (s >> 16);
    }
    !(s as u16)
}

/// The peer's answer to a frame: an ARP reply to a request for its
/// address, an echo reply to a ping of it, and nothing to anything
/// else. Padded to the 60 bytes a frame is at least, less its FCS.
pub fn answer(f: &[u8]) -> Option<Vec<u8>> {
    if f.len() < 14 {
        return None;
    }
    let (dst, src) = (&f[0..6], &f[6..12]);
    let kind = u16::from_be_bytes([f[12], f[13]]);
    let mut out = Vec::new();
    match kind {
        0x0806 if f.len() >= 42 => {
            let a = &f[14..42];
            let op = u16::from_be_bytes([a[6], a[7]]);
            if op != 1 || a[24..28] != PEER_IP {
                return None;
            }
            out.extend(src);
            out.extend(PEER_MAC);
            out.extend([0x08, 0x06, 0, 1, 0x08, 0x00, 6, 4, 0, 2]);
            out.extend(PEER_MAC);
            out.extend(PEER_IP);
            out.extend(&a[8..18]);
        }
        0x0800 if f.len() >= 34 && dst == PEER_MAC => {
            let ihl = ((f[14] & 0xf) as usize) * 4;
            let total = u16::from_be_bytes([f[16], f[17]]) as usize;
            if f[23] != 1 || f[30..34] != PEER_IP || f.len() < 14 + total {
                return None;
            }
            let ip = &f[14..14 + ihl];
            let icmp = &f[14 + ihl..14 + total];
            if icmp.first() != Some(&8) {
                return None;
            }
            out.extend(src);
            out.extend(PEER_MAC);
            out.extend([0x08, 0x00]);
            let mut h = ip.to_vec();
            h[8] = 64;
            h[10] = 0;
            h[11] = 0;
            // From the peer, to whoever pinged it.
            h[12..16].copy_from_slice(&PEER_IP);
            h[16..20].copy_from_slice(&ip[12..16]);
            let c = checksum(&h).to_be_bytes();
            h[10..12].copy_from_slice(&c);
            let mut e = icmp.to_vec();
            e[0] = 0;
            e[2] = 0;
            e[3] = 0;
            let c = checksum(&e).to_be_bytes();
            e[2..4].copy_from_slice(&c);
            out.extend(h);
            out.extend(e);
        }
        _ => return None,
    }
    out.resize(out.len().max(60), 0);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registers are at LiteX's offsets, the ones Linux's driver
    /// fixes, and the model reads and writes them as the part does.
    #[test]
    fn the_registers_are_at_litexs_offsets() {
        let want = [
            (regs::rx_slot, 0x00),
            (regs::rx_length, 0x04),
            (regs::rx_ev_pending, 0x10),
            (regs::rx_ev_enable, 0x14),
            (regs::tx_start, 0x18),
            (regs::tx_ready, 0x1c),
            (regs::tx_slot, 0x24),
            (regs::tx_length, 0x28),
            (regs::tx_ev_pending, 0x30),
            (regs::tx_ev_enable, 0x34),
        ];
        for (got, want) in want {
            assert_eq!(got, want);
        }
        let mut e = Eth::default();
        e.store(regs::rx_ev_enable, 1);
        e.store(regs::tx_slot, 1);
        e.store(regs::tx_length, 98);
        assert!(matches!(
            e.store(regs::tx_start, 1),
            Effect::Send { slot: 1, len: 98 }
        ));
        e.inbox.push_back(vec![7; 64]);
        assert_eq!(e.arrival().map(|(s, f)| (s, f.len())), Some((0, 64)));
        assert!(e.irq(), "an arrival raises the line when enabled");
        assert_eq!(e.load(regs::rx_length), 64);
        e.inbox.push_back(vec![8; 70]);
        assert!(e.arrival().is_none(), "nothing while one is pending");
        e.store(regs::rx_ev_pending, 1);
        assert!(!e.irq());
        assert_eq!(e.arrival().map(|(s, _)| s), Some(1), "the next slot");
    }

    /// The peer answers a request for its address, and a ping of it,
    /// with checksums that check.
    #[test]
    fn the_peer_answers_arp_and_ping() {
        let me = [0x00, 0x0a, 0x35, 0, 0, 1];
        let my_ip = [10, 0, 0, 1];
        let mut arp = vec![0xff; 6];
        arp.extend(me);
        arp.extend([0x08, 0x06, 0, 1, 0x08, 0, 6, 4, 0, 1]);
        arp.extend(me);
        arp.extend(my_ip);
        arp.extend([0; 6]);
        arp.extend(PEER_IP);
        let r = answer(&arp).expect("an ARP reply");
        assert_eq!(&r[0..6], &me);
        assert_eq!(&r[22..28], &PEER_MAC, "the peer's address");
        assert_eq!(&r[38..42], &my_ip);

        let mut ip = vec![0x45, 0, 0, 28, 0, 1, 0, 0, 64, 1, 0, 0];
        ip.extend(my_ip);
        ip.extend(PEER_IP);
        let c = checksum(&ip).to_be_bytes();
        ip[10..12].copy_from_slice(&c);
        let mut icmp = vec![8, 0, 0, 0, 0x12, 0x34, 0, 1];
        let c = checksum(&icmp).to_be_bytes();
        icmp[2..4].copy_from_slice(&c);
        let mut ping = PEER_MAC.to_vec();
        ping.extend(me);
        ping.extend([0x08, 0x00]);
        ping.extend(&ip);
        ping.extend(&icmp);
        let r = answer(&ping).expect("an echo reply");
        assert_eq!(&r[0..6], &me);
        assert_eq!(checksum(&r[14..34]), 0, "the IP header checks");
        assert_eq!(&r[30..34], &my_ip);
        assert_eq!(r[34], 0, "an echo reply");
        assert_eq!(checksum(&r[34..42]), 0, "the ICMP message checks");
        assert!(answer(&r).is_none(), "a reply is not answered");
    }
}
