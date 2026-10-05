// SPDX-License-Identifier: Apache-2.0
//! The doorbell: the register a program rings to start a display list
//! (issue 985).
//!
//! The rasteriser finds its own work. It reads a count at `CTRL` until
//! the count is not zero, draws that many entries, and writes the count
//! back to zero (`crate::raster`). On the board that count cannot be a
//! word of memory. In DDR3 an idle rasteriser would read it back to back
//! and take the path the scanout needs; in the data memory the loader,
//! whose data and stack live there, could leave a word that starts a
//! list nobody wrote. So it is a register of its own, on an AXI-Lite
//! port, that holds zero from reset.
//!
//! Two words:
//!
//! | offset | word |
//! |--------|------|
//! | `0x0`  | `count`: the entries to draw. A program writes it last, once the list is in memory; the rasteriser reads it and writes zero when the list is drawn. |
//! | `0x4`  | `status`: bit 0 the rasteriser's `idle` line, high while no list is being drawn. Read only. |
//!
//! A program waits for `count` to read zero, writes the list, then
//! writes `count`. A write of `count` while a list is being drawn
//! starts nothing then: the rasteriser has read the count already, and
//! writes zero over it at the end, so a program writes it only after
//! reading zero.
//!
//! The doorbell drives `ring` high while the count is not zero, and the
//! rasteriser reads the count only then, so an idle rasteriser puts no
//! reads on the bus. Polling it back to back, or even every 256
//! cycles, moved the timing of every other host's accesses.
use txhdl::comp::{Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, regmap, Trace};
use txhdl_parts::bus::axi::Resp;
use txhdl_parts::bus::axi_lite::{LiteB, LitePort, LiteR};

regmap! { doorbell (doorbell_read, doorbell_we), 1: [
    (0, count, rw, "the entries to draw; zero when there is nothing to draw", [
        (count, 0, 16, rw, 0, "the count"),
    ]),
    (1, status, ro, "what the rasteriser is doing", [
        (idle, 0, 1, ro, 1, "no list is being drawn"),
    ]),
] }

/// The doorbell's state: the count.
#[derive(Trace, Default)]
pub struct Doorbell {
    /// The entries to draw, zero when there is nothing to.
    pub count: Reg<U<16>>,
}

#[lower]
impl Unit for Doorbell {
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        (rst, idle, ring): (In<Bit>, In<Bit>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let count = self.count.get();
            // A read is answered in the cycle it is taken, and a write
            // is taken when its address and its word are both there,
            // and answered at once, as the other slots are.
            let arh = bus.ar.head();
            let awh = bus.aw.head();
            let wh = bus.w.head();
            let rsel = arh.addr.slice::<2, 1>();
            let wsel = awh.addr.slice::<2, 1>();
            let rgo = bus.r.ready() & bus.ar.peek().is_some();
            let _ = bus.ar.recv_if(bus.r.ready());
            let wgo = bus.b.ready()
                & bus.aw.peek().is_some()
                & bus.w.peek().is_some();
            let _ = bus.aw.recv_if(wgo);
            let _ = bus.w.recv_if(wgo);
            let we = doorbell_we(wgo, wsel);
            if rgo.to_bool() {
                bus.r.send(LiteR {
                    data: doorbell_read(
                        rsel,
                        doorbell_count_pack(count),
                        doorbell_status_pack(idle.get()),
                    ),
                    resp: Resp::Okay,
                });
            }
            if wgo.to_bool() {
                bus.b.send(LiteB { resp: Resp::Okay });
            }
            // The rasteriser reads the count only while it is rung.
            ring.set(Bit::from(count != 0));
            if rst.get().to_bool() {
                self.count.set(0);
            } else if we.bit(0).to_bool() {
                self.count.set(doorbell_count_count(wh.data));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;
    use txhdl::comp::{join2, signal, Running};
    use txhdl_parts::bus::axi_lite::{axi_lite, LiteAw, LiteHost, LiteW};

    type Host = LiteHost<32, 32, 4>;

    async fn write(h: &Host, addr: u32, data: u32) {
        let (aw, _, w, b, _) = h;
        aw.send(LiteAw {
            addr: U::from(addr),
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

    async fn read(h: &Host, addr: u32) -> u32 {
        let (_, ar, _, _, r) = h;
        ar.send(LiteAw {
            addr: U::from(addr),
            prot: U::from(0u8),
        });
        loop {
            DefaultClock::rising().await;
            if let Some(got) = r.recv() {
                return got.data.raw() as u32;
            }
        }
    }

    /// The count holds what a program writes, reads back, and is zero
    /// after a reset; the status is the rasteriser's line.
    #[test]
    fn the_count_holds_what_is_written_and_the_status_is_the_line() {
        let link = axi_lite::<32, 32, 4>();
        let bus: LitePort<32, 32, 4> = link.per.into();
        let host = link.host;
        let (rst_o, rst) = signal::<Bit, DefaultClock>();
        let (idle_o, idle) = signal::<Bit, DefaultClock>();
        let (ring_o, ring) = signal::<Bit, DefaultClock>();
        let done = Rc::new(Cell::new(false));
        let d = done.clone();
        let mut bell = Doorbell::default();
        let client = async move {
            let h = &host;
            assert_eq!(read(h, doorbell::count).await, 0, "zero");
            idle_o.set(Bit::One);
            assert_eq!(read(h, doorbell::status).await, 1, "idle");
            write(h, doorbell::count, 7).await;
            assert_eq!(read(h, doorbell::count).await, 7);
            assert_eq!(ring.get(), Bit::One, "rung while not zero");
            idle_o.set(Bit::Zero);
            assert_eq!(read(h, doorbell::status).await, 0, "busy");
            // The rasteriser writes zero at the end of a list.
            write(h, doorbell::count, 0).await;
            assert_eq!(read(h, doorbell::count).await, 0);
            // A reset empties it.
            write(h, doorbell::count, 3).await;
            rst_o.set(Bit::One);
            DefaultClock::rising().await;
            DefaultClock::rising().await;
            rst_o.set(Bit::Zero);
            assert_eq!(read(h, doorbell::count).await, 0, "reset");
            assert_eq!(ring.get(), Bit::Zero, "quiet at zero");
            d.set(true);
        };
        let mut sim =
            Running::new(join2(client, bell.run(bus, (rst, idle, ring_o))));
        for _ in 0..400 {
            sim.cycle();
            if done.get() {
                return;
            }
        }
        panic!("the client did not finish");
    }

    /// The doorbell lowers, one module with its count.
    #[test]
    fn the_doorbell_lowers() {
        let v = Doorbell::verilog("doorbell");
        assert!(v.contains("module doorbell("), "the module");
        assert!(v.contains("count"), "the count");
    }
}
