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
//! # No transmit interrupt
//!
//! LiteEth's driver disables it and polls `tx_ready` instead, because
//! Zephyr's send is allowed to block. So the transmit side is a ready
//! bit and a start bit, and only receive raises a line. `tx_ev_pending`
//! and `tx_ev_enable` exist because the driver acknowledges them, and
//! do nothing else.
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

use crate::bus::axi_lite::{LiteB, LitePort, LiteR};

/// A slot is this many bytes, which is `FRAME_MAX`.
pub const SLOT: usize = 2048;

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
    /// Which slot the last received frame is in.
    pub rx_slot: Reg<U<1>>,
    /// Its length in bytes.
    pub rx_length: Reg<U<16>>,
    /// A frame has arrived and not been acknowledged.
    pub rx_pending: Reg<Bit>,
    /// Whether an arrival raises the interrupt line.
    pub rx_enable: Reg<Bit>,
    /// Which slot the next transmit reads from.
    pub tx_slot: Reg<U<1>>,
    /// How many bytes of it to send.
    pub tx_length: Reg<U<16>>,
    /// Set by a write to `tx_start`, cleared when the engine takes it.
    pub tx_go: Reg<Bit>,
    /// Acknowledged by the driver and otherwise unused.
    pub tx_pending: Reg<Bit>,
    /// Whether a transmit completion would raise the line, which it
    /// never does: the driver polls `tx_ready` instead.
    pub tx_enable: Reg<Bit>,
    /// The store engine's `running` as it was at the last edge, so
    /// that its fall can be seen.
    pub rx_was: Reg<Bit>,
}
// end{state}

#[lower]
impl<const BASE: usize> Unit for EthSlots<BASE> {
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        (
            tx_busy,
            rx_busy,
            rx_len,
            rx_which,
            tx_base,
            tx_bytes,
            tx_start,
            rx_base,
            irq,
        ): (
            In<Bit>,
            In<Bit>,
            In<U<16>>,
            In<U<1>>,
            Out<U<32>>,
            Out<U<16>>,
            Out<Bit>,
            Out<U<32>>,
            Out<Bit>,
        ),
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
            let rx_ack = wgo & (wsel == 2) & data.bit(0).to_bool();
            let tx_ack = wgo & (wsel == 8) & data.bit(0).to_bool();

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
            irq.set(self.rx_pending.get() & self.rx_enable.get());

            // The acknowledgements come FIRST, and the arrival after
            // them, because `with!` applies its entries in order and
            // the last drive of a field wins. A driver acknowledging
            // one frame in the same cycle the next one lands would
            // otherwise clear the pending bit that arrival had just
            // set: the frame would sit in memory, announced to
            // nobody, and the link would stall until another arrived.
            // Written the other way round it reads more naturally and
            // loses a frame under exactly the load that makes the two
            // coincide.
            with!(self <= {
                rx_ack ? rx_pending: Bit::Zero,
                tx_ack ? tx_pending: Bit::Zero,
                arrived ? {
                    rx_slot: rx_which.get(),
                    rx_length: rx_len.get(),
                    rx_pending: Bit::One,
                },
                rx_was: now_busy,
                taken ? tx_go: Bit::Zero,
                wgo & (wsel == 3) ? rx_enable: data.bit(0),
                wgo & (wsel == 4) ? tx_slot: data.slice::<0, 1>(),
                wgo & (wsel == 5) ? tx_length: data.slice::<0, 16>(),
                wgo & (wsel == 6) ? tx_go: data.bit(0),
                wgo & (wsel == 9) ? tx_enable: data.bit(0),
            });

            if rgo.to_bool() {
                // The map, as LiteEth has it. A word not named reads
                // as zero rather than as whatever was last on the bus.
                let v0 = mux(
                    rsel == 0,
                    self.rx_slot.get().resize::<32>(),
                    mux(
                        rsel == 1,
                        self.rx_length.get().resize::<32>(),
                        mux(
                            rsel == 2,
                            mux(
                                self.rx_pending.get(),
                                U::<32>::from(1u8),
                                U::<32>::from(0u8),
                            ),
                            mux(
                                self.rx_enable.get(),
                                U::<32>::from(1u8),
                                U::<32>::from(0u8),
                            ),
                        ),
                    ),
                );
                let v1 = mux(
                    rsel == 7,
                    mux(
                        !tx_busy.get() & !self.tx_go.get(),
                        U::<32>::from(1u8),
                        U::<32>::from(0u8),
                    ),
                    mux(
                        rsel == 8,
                        mux(
                            self.tx_pending.get(),
                            U::<32>::from(1u8),
                            U::<32>::from(0u8),
                        ),
                        // Word 9 by name, so that the write-only words
                        // 4 to 6 and the unnamed 10 to 15 fall through
                        // to zero rather than mirror it (issue 454).
                        mux(
                            rsel == 9,
                            mux(
                                self.tx_enable.get(),
                                U::<32>::from(1u8),
                                U::<32>::from(0u8),
                            ),
                            U::<32>::from(0u8),
                        ),
                    ),
                );
                bus.r.send(LiteR {
                    data: mux(rsel < 4, v0, v1),
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

    /// Word 9 reads its bit, and nothing else reads it: not the three
    /// write-only words 4 to 6, which read zero rather than an unrelated
    /// bit, and not the unnamed tail 10 to 15.
    #[test]
    fn only_word_9_reads_tx_ev_enable() {
        let link = axi_lite::<32, 32, 4>();
        let bus: LitePort<32, 32, 4> = link.per.into();
        let host = link.host;
        let (_tx_busy_o, tx_busy) = signal::<Bit, DefaultClock>();
        let (_rx_busy_o, rx_busy) = signal::<Bit, DefaultClock>();
        let (_rx_len_o, rx_len) = signal::<U<16>, DefaultClock>();
        let (_rx_which_o, rx_which) = signal::<U<1>, DefaultClock>();
        let (tx_base, _) = signal::<U<32>, DefaultClock>();
        let (tx_bytes, _) = signal::<U<16>, DefaultClock>();
        let (tx_start, _) = signal::<Bit, DefaultClock>();
        let (rx_base, _) = signal::<U<32>, DefaultClock>();
        let (irq, _) = signal::<Bit, DefaultClock>();
        let seen: Rc<RefCell<Vec<(u32, u32)>>> = Rc::default();
        let log = seen.clone();
        let client = async move {
            write(&host, 9, 1).await;
            write(&host, 5, 0x5a).await;
            for word in [4, 5, 6, 9, 10, 11, 12, 13, 14, 15] {
                let v = read(&host, word).await;
                log.borrow_mut().push((word, v));
            }
        };
        let mut slots = EthSlots::<0x4100_0000>::default();
        let mut sim = Running::new(join2(
            slots.run(
                bus,
                (
                    tx_busy, rx_busy, rx_len, rx_which, tx_base, tx_bytes,
                    tx_start, rx_base, irq,
                ),
            ),
            client,
        ));
        for _ in 0..200 {
            sim.cycle();
        }
        let seen = seen.borrow();
        assert_eq!(seen.len(), 10, "every read answered");
        for (word, v) in seen.iter() {
            let want = if *word == 9 { 1 } else { 0 };
            assert_eq!(*v, want, "word {word}");
        }
    }
}
