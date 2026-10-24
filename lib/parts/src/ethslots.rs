// SPDX-License-Identifier: Apache-2.0
//! The registers a Zephyr Ethernet driver talks to.
//!
//! The map is LiteEth's, from `drivers/ethernet/eth_litex_liteeth.c`
//! in Zephyr, so that the driver is a port rather than a design. Two
//! slots per direction, ping-pong, each a flat buffer of
//! [`crate::eth::FRAME_MAX`] bytes, which is 2048 and exactly
//! LiteEth's `0x800`.
//!
//! # Where the buffers are, and why it matters
//!
//! LiteEth's slots are SRAM inside the peripheral, and its driver
//! copies each frame into them over the bus. That is the ceiling this
//! is meant to lift: a frame copied a word at a time by the processor
//! is the processor doing a memcpy it should not be doing.
//!
//! So the slots here are a region of main memory instead, at
//! `BASE`, and the peripheral fetches from it with
//! [`crate::dma::LineFetch`] and fills it with
//! [`crate::dma::LineStore`]. The driver cannot tell: it is told a
//! base address for the buffers in its device tree either way, and it
//! writes a frame there and then writes `tx_start`. What changes is
//! who moves the bytes from there to the wire.
//!
//! # The transmit event
//!
//! A transmit sets `tx_ev_pending` when the transmitter falls idle at
//! its end, and raises the line while `tx_ev_enable` is set, as
//! LiteEth's does (#1895). Zephyr's and Linux's drivers leave the
//! enable clear and poll `tx_ready` instead, because a send is allowed
//! to block; Zephyr's acknowledges the pending bit after each send, and
//! Linux's never reads it.
use txhdl::comp::{Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, regmap, with, Trace};

use crate::bus::axi_lite::{LiteB, LitePort, LiteR};
use crate::eth::ungray16;

/// A slot is this many bytes, which is `FRAME_MAX`.
pub const SLOT: usize = 2048;

// begin{regs}
// LiteEth's map at LiteX's own offsets, a word a register, so that
// Linux's `litex_liteeth`, whose offsets are fixed, drives it unchanged
// (issue 1203); Zephyr's driver takes the offsets from the header
// `//tools/regmap` writes, and follows. The writer is LiteEth's name for
// the receive side and the reader for the transmit side.
regmap! { regs (regs_read, regs_we), 4: [
    (0, rx_slot, ro, "which slot the oldest unacknowledged frame is in", [
        (slot, 0, 1, ro, 0, "the slot"),
    ]),
    (1, rx_length, ro, "its length in bytes", [
        (length, 0, 16, ro, 0, "the length"),
    ]),
    (2, rx_errors, ro, "frames dropped because both slots were pending", [
        (errors, 0, 32, ro, 0, "the count"),
    ]),
    (3, rx_ev_status, ro, "a frame has arrived and is not acknowledged", [
        (status, 0, 1, ro, 0, "the event"),
    ]),
    (4, rx_ev_pending, w1c, "a frame arrived and is not acknowledged", [
        (pending, 0, 1, w1c, 0, "set by an arrival; written one, cleared"),
    ]),
    (5, rx_ev_enable, rw, "whether an arrival raises the line", [
        (enable, 0, 1, rw, 0, "the enable"),
    ]),
    (6, tx_start, wo, "written one, the transmit starts", [
        (start, 0, 1, wo, 0, "the start"),
    ]),
    (7, tx_ready, ro, "no transmit is running or waiting", [
        (ready, 0, 1, ro, 1, "ready"),
    ]),
    (8, tx_level, ro, "transmits waiting; one at a time, so zero", [
        (level, 0, 2, ro, 0, "the level"),
    ]),
    (9, tx_slot, wo, "which slot the next transmit reads", [
        (slot, 0, 1, wo, 0, "the slot"),
    ]),
    (10, tx_length, wo, "how many bytes of it to send", [
        (length, 0, 16, wo, 0, "the length"),
    ]),
    (11, tx_ev_status, ro, "a transmit has ended and is not acknowledged", [
        (status, 0, 1, ro, 0, "the event"),
    ]),
    (12, tx_ev_pending, w1c, "a transmit ended and is not acknowledged", [
        (pending, 0, 1, w1c, 0, "set as a transmit ends; written one, cleared"),
    ]),
    (13, tx_ev_enable, rw, "whether a transmit's end raises the line", [
        (enable, 0, 1, rw, 0, "the enable"),
    ]),
    (14, rx_mac_check, ro, "frames the MAC dropped for their check", [
        (count, 0, 16, ro, 0, "a failed check sequence or a receive error"),
    ]),
    (15, rx_mac_drops, ro, "frames the MAC dropped for room or size", [
        (room, 0, 16, ro, 0, "both of the MAC's frames were held"),
        (size, 16, 16, ro, 0, "four bytes or fewer, or longer than its store"),
    ]),
] }
// end{regs}

// begin{state}
/// The slot registers, as LiteEth lays them out.
///
/// `BASE` is where the four buffers start, and a slot is `BASE +
/// n * 2048`.
///
/// `BASE` must be 1 KB aligned, and nothing here checks it. A slot is
/// then 1 KB aligned too, since the slots are 2 KB apart, and a 256
/// beat burst of words is exactly 1 KB, so no burst can cross AXI4's
/// 4 KB boundary. A `BASE` that is not aligned puts that guarantee
/// back on the caller, and the burst that straddles a boundary is a
/// protocol violation rather than a wrong address, so it will not
/// look like a bug here.
#[derive(Trace, Default)]
pub struct EthSlots<const BASE: usize> {
    /// The oldest frame received and not yet acknowledged: its slot.
    /// The frames wait in order, at most one a slot (issue 1313).
    /// `rx_slot` and `rx_length` read the oldest, and acknowledging it
    /// takes it off, so a frame that lands while the driver is busy
    /// with another waits its turn rather than replacing it.
    pub q0_slot: Reg<U<1>>,
    /// The oldest frame's length in bytes.
    pub q0_len: Reg<U<16>>,
    /// The frame behind it, if one is: its slot.
    pub q1_slot: Reg<U<1>>,
    /// Its length in bytes.
    pub q1_len: Reg<U<16>>,
    /// How many of the two are held.
    pub queued: Reg<U<2>>,
    /// Whether an arrival raises the interrupt line.
    pub rx_enable: Reg<Bit>,
    /// Which slot the next transmit reads from.
    pub tx_slot: Reg<U<1>>,
    /// How many bytes of it to send.
    pub tx_length: Reg<U<16>>,
    /// Set by a write to `tx_start`, cleared when the engine takes it.
    pub tx_go: Reg<Bit>,
    /// A transmit has ended and the driver has not acknowledged it.
    pub tx_pending: Reg<Bit>,
    /// Whether a transmit's end raises the line.
    pub tx_enable: Reg<Bit>,
    /// The transmitter's `tx_busy` as it was at the last edge, so that
    /// its fall, the end of a transmit, can be seen.
    pub tx_was: Reg<Bit>,
    /// The store engine's `running` as it was at the last edge, so
    /// that its fall can be seen.
    pub rx_was: Reg<Bit>,
}
// end{state}

/// The slots' link to the MAC and the core (issue 726), taken apart in
/// `run`, so each port keeps its name.
pub struct SlotsMac {
    /// The transmitter is sending a frame.
    pub tx_busy: In<Bit>,
    /// The receive store engine is storing a frame.
    pub rx_busy: In<Bit>,
    /// The length of the frame last stored.
    pub rx_len: In<U<16>>,
    /// The slot the frame last stored landed in.
    pub rx_which: In<U<1>>,
    /// The frames the store engine dropped.
    pub rx_drops: In<U<32>>,
    /// The MAC's own drop counts, by cause, packed.
    pub rx_mac: In<U<48>>,
    /// Where the frame to send starts.
    pub tx_base: Out<U<32>>,
    /// The length of the frame to send.
    pub tx_bytes: Out<U<16>>,
    /// Send it.
    pub tx_start: Out<Bit>,
    /// Where the next frame received is to land.
    pub rx_base: Out<U<32>>,
    /// The interrupt line to the core.
    pub irq: Out<Bit>,
    /// Every receive slot is full.
    pub rx_full: Out<Bit>,
}

#[lower]
impl<const BASE: usize> Unit for EthSlots<BASE> {
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        SlotsMac {
            tx_busy,
            rx_busy,
            rx_len,
            rx_which,
            rx_drops,
            rx_mac,
            tx_base,
            tx_bytes,
            tx_start,
            rx_base,
            irq,
            rx_full,
        }: SlotsMac,
    ) {
        loop {
            DefaultClock::rising().await;
            let arh = bus.ar.head();
            let awh = bus.aw.head();
            let wh = bus.w.head();
            let rsel = arh.addr.slice::<2, 4>();
            let wsel = awh.addr.slice::<2, 4>();
            let rgo = bus.r.ready() & bus.ar.peek().is_some();
            let _ = bus.ar.recv_if(bus.r.ready());
            let wgo = bus.b.ready()
                & bus.aw.peek().is_some()
                & bus.w.peek().is_some();
            let _ = bus.aw.recv_if(wgo);
            let _ = bus.w.recv_if(wgo);
            let data = wh.data;

            // The engine took the request when it went busy, so the
            // start pulse lasts until it does.
            let busy = tx_busy.get();
            let go = self.tx_go.get();
            let taken = go & busy;
            // A transmit ends when the transmitter falls idle, as an
            // arrival is the store engine's fall (#1895).
            let sent = self.tx_was.get() & !busy;

            // An arrival is the store engine falling idle, not the
            // frame being received. `LineStore` holds `running` high
            // until the write response comes back from memory, so its
            // fall is the first moment the frame is certainly in
            // memory. Raising the interrupt on the receiver instead
            // would tell the driver to read a frame whose last beats
            // are still in flight.
            //
            // Taking `running` rather than a `done` pulse is
            // deliberate: it makes the correct wiring the only
            // wiring. A `done` port could be joined to the receiver
            // by someone reading the map and not this comment.
            let now_busy = rx_busy.get();
            let arrived = self.rx_was.get() & !now_busy;
            // Writing a one to a pending bit clears it, which is what
            // `RW1C` means and what every Zephyr driver does to
            // acknowledge.
            // The write enables, a bit a register, which seven of the
            // updates below take a bit off.
            let we = regs_we(wgo, wsel);
            let rx_ack = we.bit(4) & regs_rx_ev_pending_pending(data);
            let tx_ack = we.bit(12) & regs_tx_ev_pending_pending(data);

            // A slot's address. The two directions are separate
            // regions, receive first and transmit 4096 bytes above
            // it, because a slot number alone would make transmit
            // slot zero and receive slot zero the same address and a
            // frame arriving would land on one waiting to go out.
            // Written out rather than through a helper because the
            // lowering takes expressions and not closures.
            let base = U::<32>::from(BASE as u32);
            let tx_region = base + U::<32>::from(0x1000u32);
            tx_base
                .set(tx_region + (self.tx_slot.get().resize::<32>() << 11u32));
            tx_bytes.set(self.tx_length.get());
            tx_start.set(go & !busy);
            // The receiving side is told where the slot it is filling
            // begins; which slot that is comes from the far side, so
            // that a frame lands somewhere the driver is not reading.
            rx_base.set(base + (rx_which.get().resize::<32>() << 11u32));

            // The frames waiting for the driver, a slot each (issue
            // 1313). The pending bit is held while any waits, so a
            // driver that acknowledges one frame an interrupt, as
            // Zephyr's and Linux's do, is interrupted again for the
            // next. The receiving side is told the slots are full while
            // both hold a frame, or while one does and the frame just
            // stored is about to be counted, and then drops the frame
            // it is offered and counts it, rather than land it on a
            // slot the driver has not released.
            let q = self.queued.get();
            let none = Bit::from(q == 0);
            let one = Bit::from(q == 1);
            let two = Bit::from(q == 2);
            let held = !none;
            irq.set(
                (held & self.rx_enable.get())
                    | (self.tx_pending.get() & self.tx_enable.get()),
            );
            rx_full.set(two | (one & self.rx_was.get()));

            // An acknowledgement takes the oldest frame off, and an
            // arrival puts the new one behind whatever is left, so the
            // two in the same cycle move the second frame to the front
            // and put the new one behind it. Before issue 1313 an
            // arrival replaced the frame the driver had been told of,
            // and its acknowledgement cleared the arrival's pending bit.
            let pop = rx_ack & held;
            let new_slot = rx_which.get();
            let new_len = rx_len.get();
            with!(self <= {
                pop & two ? {
                    q0_slot: self.q1_slot.get(),
                    q0_len: self.q1_len.get(),
                },
                arrived & (none | (one & pop)) ? {
                    q0_slot: new_slot,
                    q0_len: new_len,
                },
                arrived & ((one & !pop) | (two & pop)) ? {
                    q1_slot: new_slot,
                    q1_len: new_len,
                },
                arrived & !pop & !two ? queued: q + 1,
                pop & !arrived ? queued: q - 1,
                sent ? tx_pending: Bit::One,
                tx_ack & !sent ? tx_pending: Bit::Zero,
                tx_was: busy,
                rx_was: now_busy,
                taken ? tx_go: Bit::Zero,
                we.bit(5) ? rx_enable: regs_rx_ev_enable_enable(data),
                we.bit(6) ? tx_go: regs_tx_start_start(data),
                we.bit(9) ? tx_slot: regs_tx_slot_slot(data).zext::<1>(),
                we.bit(10) ? tx_length: regs_tx_length_length(data),
                we.bit(13) ? tx_enable: regs_tx_ev_enable_enable(data),
            });

            if rgo.to_bool() {
                // The map, as LiteEth has it: the write-only words and
                // the counts nothing here keeps read as zero rather than
                // mirror a neighbour (issue 454). 14 and 15, which
                // LiteEth leaves unnamed, are the MAC's drops ahead of
                // the slots, crossed from its clock in Gray code and
                // counted here (issue 1404).
                let mac = rx_mac.get();
                let ready = !tx_busy.get() & !self.tx_go.get();
                let zero = U::<32>::from(0u8);
                let word = regs_read(
                    rsel,
                    regs_rx_slot_pack(self.q0_slot.get().bit(0)),
                    regs_rx_length_pack(self.q0_len.get()),
                    regs_rx_errors_pack(rx_drops.get()),
                    regs_rx_ev_status_pack(held),
                    regs_rx_ev_pending_pack(held),
                    regs_rx_ev_enable_pack(self.rx_enable.get()),
                    zero,
                    regs_tx_ready_pack(ready),
                    zero,
                    zero,
                    zero,
                    regs_tx_ev_status_pack(self.tx_pending.get()),
                    regs_tx_ev_pending_pack(self.tx_pending.get()),
                    regs_tx_ev_enable_pack(self.tx_enable.get()),
                    regs_rx_mac_check_pack(ungray16(mac.slice::<16, 16>())),
                    regs_rx_mac_drops_pack(
                        ungray16(mac.slice::<32, 16>()),
                        ungray16(mac.slice::<0, 16>()),
                    ),
                );
                bus.r.send(LiteR {
                    data: word,
                    resp: crate::bus::axi::Resp::Okay,
                });
            }
            if wgo.to_bool() {
                bus.b.send(LiteB {
                    resp: crate::bus::axi::Resp::Okay,
                });
            }
        }
    }
}

/// The map read with `tx_ev_enable` set, which is what issue 454 found
/// mirrored into every word the read path did not name.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::axi::Resp;
    use crate::bus::axi_lite::{axi_lite, LiteAr, LiteAw, LiteHost, LiteW};
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, signal, Running};

    type Host = LiteHost<32, 32, 4>;

    async fn write(h: &Host, word: u32, data: u32) {
        let (aw, _, w, b, _) = h;
        aw.send(LiteAw {
            addr: U::from(4 * word),
            prot: U::from(0u8),
        });
        w.send(LiteW {
            data: U::from(data),
            strb: U::from(0xfu8),
        });
        loop {
            DefaultClock::rising().await;
            if b.recv().is_some() {
                return;
            }
        }
    }

    async fn read(h: &Host, word: u32) -> u32 {
        let (_, ar, _, _, r) = h;
        ar.send(LiteAr {
            addr: U::from(4 * word),
            prot: U::from(0u8),
        });
        loop {
            DefaultClock::rising().await;
            if let Some(v) = r.recv() {
                assert!(matches!(v.resp, Resp::Okay));
                return v.data.raw() as u32;
            }
        }
    }

    /// Word 13 reads its bit, and nothing else reads it: not the
    /// write-only words 6, 9 and 10, which read zero rather than an
    /// unrelated bit, not the counts at 2, 8 and 11, which read zero
    /// here (word 2 reads `rx_drops`, which this test never drives,
    /// and nothing keeps 8 or 11), and not the MAC's drop counts at 14
    /// and 15, which LiteEth leaves unnamed. LiteX's offsets (issue
    /// 1203); the slot length written to word 10 must not show either.
    #[test]
    fn only_word_13_reads_tx_ev_enable() {
        let link = axi_lite::<32, 32, 4>();
        let bus: LitePort<32, 32, 4> = link.per.into();
        let host = link.host;
        let (_tx_busy_o, tx_busy) = signal::<Bit, DefaultClock>();
        let (_rx_busy_o, rx_busy) = signal::<Bit, DefaultClock>();
        let (_rx_len_o, rx_len) = signal::<U<16>, DefaultClock>();
        let (_rx_which_o, rx_which) = signal::<U<1>, DefaultClock>();
        let (_rx_drops_o, rx_drops) = signal::<U<32>, DefaultClock>();
        // The MAC's drops by cause, in Gray code as they cross: room 3,
        // check 5, size 7 (issue 1404).
        let (rx_mac_o, rx_mac) = signal::<U<48>, DefaultClock>();
        let g = |x: u32| crate::eth::gray16(U::<16>::from(x)).raw();
        rx_mac_o.set(U::<48>::from((g(3) << 32) | (g(5) << 16) | g(7)));
        let (tx_base, _) = signal::<U<32>, DefaultClock>();
        let (tx_bytes, _) = signal::<U<16>, DefaultClock>();
        let (tx_start, _) = signal::<Bit, DefaultClock>();
        let (rx_base, _) = signal::<U<32>, DefaultClock>();
        let (irq, _) = signal::<Bit, DefaultClock>();
        let (rx_full, _) = signal::<Bit, DefaultClock>();
        let seen: Rc<RefCell<Vec<(u32, u32)>>> = Rc::default();
        let log = seen.clone();
        let client = async move {
            write(&host, 13, 1).await;
            write(&host, 10, 0x5a).await;
            for word in [2, 6, 8, 9, 10, 11, 12, 13, 14, 15] {
                let v = read(&host, word).await;
                log.borrow_mut().push((word, v));
            }
        };
        let mut slots = EthSlots::<0x4100_0000>::default();
        let mut sim = Running::new(join2(
            slots.run(
                bus,
                SlotsMac {
                    tx_busy,
                    rx_busy,
                    rx_len,
                    rx_which,
                    rx_drops,
                    rx_mac,
                    tx_base,
                    tx_bytes,
                    tx_start,
                    rx_base,
                    irq,
                    rx_full,
                },
            ),
            client,
        ));
        for _ in 0..200 {
            sim.cycle();
        }
        let seen = seen.borrow();
        assert_eq!(seen.len(), 10, "every read answered");
        for (word, v) in seen.iter() {
            let want = match *word {
                13 => 1,
                // The MAC's drops, decoded: check, then size over room.
                14 => 5,
                15 => (7 << 16) | 3,
                _ => 0,
            };
            assert_eq!(*v, want, "word {word}");
        }
    }

    /// A transmit sets its event as the transmitter falls idle, and
    /// the event raises the line only while `tx_ev_enable` is set
    /// (#1895): `tx_ev_status` and `tx_ev_pending` read one after a
    /// send, the line stays low until the enable is written, and
    /// acknowledging the event clears the bit and the line.
    #[test]
    fn a_transmit_sets_its_event_and_raises_the_line_when_enabled() {
        let link = axi_lite::<32, 32, 4>();
        let bus: LitePort<32, 32, 4> = link.per.into();
        let host = link.host;
        let (tx_busy_o, tx_busy) = signal::<Bit, DefaultClock>();
        let (_rx_busy_o, rx_busy) = signal::<Bit, DefaultClock>();
        let (_rx_len_o, rx_len) = signal::<U<16>, DefaultClock>();
        let (_rx_which_o, rx_which) = signal::<U<1>, DefaultClock>();
        let (_rx_drops_o, rx_drops) = signal::<U<32>, DefaultClock>();
        let (_rx_mac_o, rx_mac) = signal::<U<48>, DefaultClock>();
        let (tx_base, _) = signal::<U<32>, DefaultClock>();
        let (tx_bytes, _) = signal::<U<16>, DefaultClock>();
        let (tx_start, _) = signal::<Bit, DefaultClock>();
        let (rx_base, _) = signal::<U<32>, DefaultClock>();
        let (irq, irq_in) = signal::<Bit, DefaultClock>();
        let (rx_full, _) = signal::<Bit, DefaultClock>();
        let done = Rc::new(RefCell::new(false));
        let d = done.clone();
        let client = async move {
            let line = || irq_in.get().to_bool();
            assert_eq!(read(&host, 12).await, 0, "nothing sent yet");
            // A transmit: the transmitter busy for ten cycles, then idle.
            tx_busy_o.set(Bit::One);
            for _ in 0..10 {
                DefaultClock::rising().await;
            }
            tx_busy_o.set(Bit::Zero);
            for _ in 0..3 {
                DefaultClock::rising().await;
            }
            assert_eq!(read(&host, 11).await, 1, "the event's status");
            assert_eq!(read(&host, 12).await, 1, "the event is pending");
            assert!(!line(), "the line stays low while disabled");
            write(&host, 13, 1).await;
            DefaultClock::rising().await;
            assert!(line(), "enabled, the pending event raises the line");
            // Disabled again, the line falls and the event stays.
            write(&host, 13, 0).await;
            DefaultClock::rising().await;
            assert!(!line(), "disabled, the line falls");
            assert_eq!(read(&host, 12).await, 1, "and the event stays");
            write(&host, 13, 1).await;
            DefaultClock::rising().await;
            assert!(line(), "enabled again, it rises");
            write(&host, 12, 1).await;
            DefaultClock::rising().await;
            assert_eq!(read(&host, 12).await, 0, "acknowledged");
            assert_eq!(read(&host, 11).await, 0, "its status too");
            assert!(!line(), "and the line falls with it");
            // An acknowledgement written in the cycle a transmit ends:
            // the end wins, so the event is not lost. The transmitter
            // falls and the write is offered in the same step, so the
            // slots see both at the same edge.
            let (aw, _, w, b, _) = &host;
            tx_busy_o.set(Bit::One);
            for _ in 0..10 {
                DefaultClock::rising().await;
            }
            tx_busy_o.set(Bit::Zero);
            aw.send(LiteAw {
                addr: U::from(4 * 12u32),
                prot: U::from(0u8),
            });
            w.send(LiteW {
                data: U::from(1u32),
                strb: U::from(0xfu8),
            });
            while b.recv().is_none() {
                DefaultClock::rising().await;
            }
            assert_eq!(read(&host, 12).await, 1, "the end outlives the ack");
            *d.borrow_mut() = true;
        };
        let mut slots = EthSlots::<0x4100_0000>::default();
        let mut sim = Running::new(join2(
            slots.run(
                bus,
                SlotsMac {
                    tx_busy,
                    rx_busy,
                    rx_len,
                    rx_which,
                    rx_drops,
                    rx_mac,
                    tx_base,
                    tx_bytes,
                    tx_start,
                    rx_base,
                    irq,
                    rx_full,
                },
            ),
            client,
        ));
        for _ in 0..400 {
            sim.cycle();
        }
        assert!(*done.borrow(), "the client finished");
    }
}
