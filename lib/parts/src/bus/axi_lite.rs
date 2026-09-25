// SPDX-License-Identifier: Apache-2.0
//! AXI4-Lite, and the bridge that puts small peripherals behind an
//! AXI4 link.
//!
//! AXI4-Lite is AXI4 with everything a small peripheral does not need
//! taken out: no identifier, no burst, no `last`, so one transaction
//! is one beat on each channel it uses. A peripheral on it is a few
//! lines of hardware with no tracker in front of it: a read is an
//! address in and a word out, and a write is an address and a word in
//! and a response out.
//!
//! [`axi_lite`] makes the five channels of a link, as
//! [`axi_units`](super::axi::axi_units) does for AXI4, and hands back
//! the ends each side holds. `LiteBridge1` to `LiteBridge8` sit
//! between an AXI4 link and that many AXI-Lite peripherals: they take
//! the AXI4 side one burst at a time, decode the burst's address to a
//! peripheral, send each beat to it as one AXI-Lite transaction, and
//! answer the burst with its identifier. One count of peripherals is
//! one unit, written out by `lite_bridge!`, since the lowering reads a
//! body and not a loop over ports.
//!
//! The widths are stated, as everywhere in this library: `A` the
//! address width, `D` the data width and `S` the strobe width, which
//! is `D / 8`.
use crate::bus::axi::Resp;
use txhdl::comp::{chan, DefaultClock, Link, Rx, Tx};
use txhdl::lite_bridge;
use txhdl::types::U;
use txhdl::{
    Ports as PortsDerive, Transaction as TransactionDerive,
    Value as ValueDerive,
};

// begin{beats}
/// The address phase of an AXI-Lite transaction, on either address
/// channel: the address, and the protection bits AXI4 calls `AxPROT`.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct LiteAddr<const A: usize> {
    /// The address of the word read or written.
    pub addr: U<A>,
    /// Privilege, security and whether this is an instruction fetch.
    pub prot: U<3>,
}

/// The write address channel's beat.
pub type LiteAw<const A: usize> = LiteAddr<A>;
/// The read address channel's beat. The same fields.
pub type LiteAr<const A: usize> = LiteAddr<A>;

/// The write data channel's beat: the word, and which of its bytes
/// are meant.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct LiteW<const D: usize, const S: usize> {
    /// The word written.
    pub data: U<D>,
    /// A bit per byte lane; a lane whose bit is low is not written.
    pub strb: U<S>,
}

/// The write response channel's beat: how the write went.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct LiteB {
    /// How it went.
    pub resp: Resp,
}

/// The read data channel's beat: the word read, and how the read went.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct LiteR<const D: usize> {
    /// The word read.
    pub data: U<D>,
    /// How it went.
    pub resp: Resp,
}
// end{beats}

/// The ends a host holds on an AXI-Lite link, in the order of the
/// channels: the two address channels and the write data it drives,
/// and the write response and read data it reads.
pub type LiteHost<const A: usize, const D: usize, const S: usize> = (
    Tx<LiteAw<A>>,
    Tx<LiteAr<A>>,
    Tx<LiteW<D, S>>,
    Rx<LiteB>,
    Rx<LiteR<D>>,
);

/// The ends a peripheral holds on an AXI-Lite link, in the same order:
/// what it reads, and then what it drives.
pub type LitePer<const A: usize, const D: usize, const S: usize> = (
    Rx<LiteAw<A>>,
    Rx<LiteAr<A>>,
    Rx<LiteW<D, S>>,
    Tx<LiteB>,
    Tx<LiteR<D>>,
);

/// What a peripheral holds on an AXI-Lite link, as one port rather
/// than five: `run(&mut self, bus: LitePort<32, 32, 4>, ..)` and
/// `bus.ar` in the body. Each field is one channel, its beat one of
/// the structs above, and the netlist names each port for the side and
/// the field, `bus_aw_valid`, `bus_r_data` and so on, as AXI's own
/// signals are named with a prefix. The fields are in [`LitePer`]'s
/// order. Any unit in any crate may take it, since `#[derive(Ports)]`
/// is all `#[lower]` needs of it (issue 483).
#[derive(PortsDerive)]
pub struct LitePort<const A: usize, const D: usize, const S: usize> {
    /// Write addresses, in.
    pub aw: Rx<LiteAw<A>>,
    /// Read addresses, in.
    pub ar: Rx<LiteAr<A>>,
    /// Write data, in.
    pub w: Rx<LiteW<D, S>>,
    /// Write responses, out.
    pub b: Tx<LiteB>,
    /// Read data, out.
    pub r: Tx<LiteR<D>>,
}

impl<const A: usize, const D: usize, const S: usize> From<LitePer<A, D, S>>
    for LitePort<A, D, S>
{
    fn from((aw, ar, w, b, r): LitePer<A, D, S>) -> Self {
        LitePort { aw, ar, w, b, r }
    }
}

/// What a host holds on an AXI-Lite link, as one port: [`LitePort`]'s
/// five channels by the same names, each end turned round. `link` makes
/// the two at once: `let (host, per) = link::<LitePort<32, 32, 4>>()`
/// (issue 498).
#[derive(PortsDerive)]
pub struct LiteHostPort<const A: usize, const D: usize, const S: usize> {
    /// Write addresses, out.
    pub aw: Tx<LiteAw<A>>,
    /// Read addresses, out.
    pub ar: Tx<LiteAr<A>>,
    /// Write data, out.
    pub w: Tx<LiteW<D, S>>,
    /// Write responses, in.
    pub b: Rx<LiteB>,
    /// Read data, in.
    pub r: Rx<LiteR<D>>,
}

impl<const A: usize, const D: usize, const S: usize> Link
    for LitePort<A, D, S>
{
    type Host = LiteHostPort<A, D, S>;
    fn link() -> (Self::Host, Self) {
        let LiteLink { host, per } = axi_lite::<A, D, S>();
        let (aw, ar, w, b, r) = host;
        (LiteHostPort { aw, ar, w, b, r }, per.into())
    }
}

/// An AXI-Lite link: the ends of its five channels, by side.
pub struct LiteLink<const A: usize, const D: usize, const S: usize> {
    /// What the host holds: a bridge's `aw`, `ar` and `w` it drives
    /// and `b` and `r` it reads for this peripheral.
    pub host: LiteHost<A, D, S>,
    /// What the peripheral holds.
    pub per: LitePer<A, D, S>,
}

/// Make an AXI-Lite link: its five channels, and the ends each side
/// holds, the way [`chan`] makes a channel and hands back two.
///
/// [`chan`]: txhdl::comp::chan
pub fn axi_lite<const A: usize, const D: usize, const S: usize>(
) -> LiteLink<A, D, S> {
    let (aw_tx, aw_rx) = chan::<LiteAw<A>, DefaultClock>();
    let (ar_tx, ar_rx) = chan::<LiteAr<A>, DefaultClock>();
    let (w_tx, w_rx) = chan::<LiteW<D, S>, DefaultClock>();
    let (b_tx, b_rx) = chan::<LiteB, DefaultClock>();
    let (r_tx, r_rx) = chan::<LiteR<D>, DefaultClock>();
    LiteLink {
        host: (aw_tx, ar_tx, w_tx, b_rx, r_rx),
        per: (aw_rx, ar_rx, w_rx, b_tx, r_tx),
    }
}

// begin{part}
lite_bridge!(LiteBridge1, 1);
lite_bridge!(LiteBridge2, 2);
lite_bridge!(LiteBridge3, 3);
lite_bridge!(LiteBridge4, 4);
lite_bridge!(LiteBridge5, 5);
lite_bridge!(LiteBridge6, 6);
lite_bridge!(LiteBridge7, 7);
lite_bridge!(LiteBridge8, 8);
// end{part}

/// The bridge against a model: bursts of one to four beats, reads and
/// writes, incrementing and fixed, to two peripherals and to a hole.
/// Every read reads what the writes left, a burst to the hole is
/// answered `DecErr` in as many beats as it asked for, and every beat
/// of a burst reaches its peripheral as a transaction of its own.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::axi::{axi, AxiHost, BurstKind, Link, Rd, Wr};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use txhdl::comp::{join2, Clock, Running, Unit};

    type Host = AxiHost<16, 32, 4, 2, 4>;
    /// Two peripherals, a nibble each; everything else is a hole.
    type Bridge = LiteBridge2<16, 32, 4, 2, 0x1000, 0xf000, 0x2000, 0xf000>;

    /// A memory behind an AXI-Lite link, written as a simulation: it
    /// counts the transactions it serves, so the test can see that a
    /// burst arrived a beat at a time.
    async fn memory(
        per: LitePer<16, 32, 4>,
        words: Rc<RefCell<HashMap<u128, u128>>>,
        served: Rc<RefCell<usize>>,
    ) {
        let (aw, ar, w, b, r) = per;
        loop {
            DefaultClock::rising().await;
            if aw.peek().is_some() && w.peek().is_some() && b.ready().to_bool()
            {
                let a = aw.recv().unwrap();
                let d = w.recv().unwrap();
                words.borrow_mut().insert(a.addr.raw(), d.data.raw());
                b.send(LiteB { resp: Resp::Okay });
                *served.borrow_mut() += 1;
            }
            if ar.peek().is_some() && r.ready().to_bool() {
                let a = ar.recv().unwrap();
                let data = *words.borrow().get(&a.addr.raw()).unwrap_or(&0);
                r.send(LiteR {
                    data: U::from(data),
                    resp: Resp::Okay,
                });
                *served.borrow_mut() += 1;
            }
        }
    }

    #[test]
    fn bursts_cross_the_bridge_a_beat_at_a_time() {
        let Link {
            host,
            host_in,
            host_out,
            per_in,
            per_out,
            ..
        } = axi::<16, 32, 4, 2, 4>();
        let l0 = axi_lite::<16, 32, 4>();
        let l1 = axi_lite::<16, 32, 4>();
        let (aw0, ar0, w0, b0, r0) = l0.host;
        let (aw1, ar1, w1, b1, r1) = l1.host;
        let words = Rc::new(RefCell::new(HashMap::new()));
        let served0 = Rc::new(RefCell::new(0usize));
        let served1 = Rc::new(RefCell::new(0usize));
        let out = Rc::new(RefCell::new(Vec::<String>::new()));
        let o = out.clone();
        // Bursts of whole words, four bytes a beat, so the address
        // moves by four.
        let words_at = |a: u32, n: usize| {
            let mut rd = Rd::at(a, n);
            rd.size = U::from(2u8);
            rd
        };
        let write_at = |a: u32| {
            let mut wr = Wr::at(a);
            wr.size = U::from(2u8);
            wr
        };
        let client = async move {
            // Three words written at 0x1004 in one incrementing burst,
            // then read back in one burst of four from 0x1000.
            let vs = [U::from(0x11u32), U::from(0x22u32), U::from(0x33u32)];
            let wr = host.write(write_at(0x1004), &vs).await;
            assert_eq!(wr.done().await.resp, Resp::Okay);
            let rd = host.read(words_at(0x1000, 4)).await;
            let got = rd.done().await;
            assert_eq!(got.resp, Resp::Okay);
            let raw: Vec<u128> = got.data.iter().map(|d| d.raw()).collect();
            assert_eq!(raw, vec![0, 0x11, 0x22, 0x33]);
            // A fixed burst reads one word three times.
            let mut fixed = words_at(0x1008, 3);
            fixed.burst = BurstKind::Fixed;
            let got = host.read(fixed).await.done().await;
            let raw: Vec<u128> = got.data.iter().map(|d| d.raw()).collect();
            assert_eq!(raw, vec![0x22, 0x22, 0x22]);
            // The second peripheral, one word.
            let wr = host.write(Wr::at(0x2000u32), &[U::from(7u32)]).await;
            assert_eq!(wr.done().await.resp, Resp::Okay);
            let got = host.read(Rd::at(0x2000u32, 1)).await.done().await;
            assert_eq!(got.data[0].raw(), 7);
            // A hole: a write of two beats and a read of two, answered
            // by the bridge.
            let two = [U::from(1u32), U::from(2u32)];
            let wr = host.write(Wr::at(0x5000u32), &two).await;
            assert_eq!(wr.done().await.resp, Resp::DecErr);
            let got = host.read(Rd::at(0x5000u32, 2)).await.done().await;
            assert_eq!(got.resp, Resp::DecErr);
            assert_eq!(got.data.len(), 2, "a read of a hole, in beats");
            o.borrow_mut().push("done".to_string());
        };
        let mut h = Host::default();
        let mut bridge = Bridge::default();
        let (s0, s1) = (served0.clone(), served1.clone());
        let mut sim = Running::new(join2(
            join2(
                h.run(host_in, host_out),
                bridge.run(
                    (per_in.0, per_in.1, per_in.2, b0, r0, b1, r1),
                    (aw0, ar0, w0, aw1, ar1, w1, per_out.2, per_out.3),
                ),
            ),
            join2(
                client,
                join2(
                    memory(l0.per, words.clone(), s0),
                    memory(l1.per, words.clone(), s1),
                ),
            ),
        ));
        for _ in 0..400 {
            sim.cycle();
        }
        assert_eq!(*out.borrow(), vec!["done".to_string()], "the run ended");
        // Three beats written, four read, three read again: ten
        // transactions on the first peripheral, and two on the second.
        assert_eq!(*served0.borrow(), 10, "one transaction per beat");
        assert_eq!(*served1.borrow(), 2, "the second peripheral's");
    }
}
