// SPDX-License-Identifier: Apache-2.0
//! What `#[unregistered]` refuses (issue 1293): a ring of children
//! joined through wires the whole way round, refused when the unit is
//! lowered; and a receiver joined before its sender, refused when the
//! unit runs. A ring that a register breaks is lowered.
use txhdl::comp::{
    chan, join2, Clock, DefaultClock, Reg, Running, Rx, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

/// A forwarding stage: its input's head to its output in the same
/// step, so its input reaches its output through wires.
#[derive(Trace, Default)]
pub struct Fwd {}

#[lower]
impl Unit for Fwd {
    async fn run(&mut self, inp: Rx<U<8>>, out: Tx<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let h = inp.head();
            let go = inp.peek().is_some() & out.ready();
            let _ = inp.recv_if(out.ready());
            if go.to_bool() {
                out.send(h);
            }
        }
    }
}

/// A stage that answers from a register, so its input does not reach
/// its output in the cycle.
#[derive(Trace, Default)]
pub struct Held {
    pub v: Reg<U<8>>,
    pub full: Reg<Bit>,
}

#[lower]
impl Unit for Held {
    async fn run(&mut self, inp: Rx<U<8>>, out: Tx<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let send = self.full.get() & out.ready();
            if send.to_bool() {
                out.send(self.v.get());
            }
            let take = inp.peek().is_some() & (!self.full.get() | out.ready());
            let q = inp.head();
            let _ = inp.recv_if(take);
            with!(self <= {
                take ? {
                    v: q,
                    full: Bit::One,
                } else {
                    send ? full: Bit::Zero,
                },
            });
        }
    }
}

/// Two forwarding stages each feeding the other through an
/// unregistered channel: a loop of wires.
#[derive(Trace, Default)]
pub struct Ring {
    pub a: Fwd,
    pub b: Fwd,
}

#[lower]
impl Unit for Ring {
    async fn run(&mut self, _i: (), _o: ()) {
        #[unregistered]
        let (ab_tx, ab_rx) = chan::<U<8>, DefaultClock>();
        #[unregistered]
        let (ba_tx, ba_rx) = chan::<U<8>, DefaultClock>();
        join2(self.a.run(ba_rx, ab_tx), self.b.run(ab_rx, ba_tx)).await;
    }
}

/// The same ring with a register in it: a forwarding stage and a held
/// one, both channels unregistered.
#[derive(Trace, Default)]
pub struct HeldRing {
    pub a: Fwd,
    pub b: Held,
}

#[lower]
impl Unit for HeldRing {
    async fn run(&mut self, _i: (), _o: ()) {
        #[unregistered]
        let (ab_tx, ab_rx) = chan::<U<8>, DefaultClock>();
        #[unregistered]
        let (ba_tx, ba_rx) = chan::<U<8>, DefaultClock>();
        join2(self.a.run(ba_rx, ab_tx), self.b.run(ab_rx, ba_tx)).await;
    }
}

/// A forwarding stage into a held one through an unregistered
/// channel, the receiver joined first.
#[derive(Trace, Default)]
pub struct Backwards {
    pub a: Fwd,
    pub b: Held,
}

#[lower]
impl Unit for Backwards {
    async fn run(&mut self, inp: Rx<U<8>>, out: Tx<U<8>>) {
        #[unregistered]
        let (ab_tx, ab_rx) = chan::<U<8>, DefaultClock>();
        join2(self.b.run(ab_rx, out), self.a.run(inp, ab_tx)).await;
    }
}

#[test]
#[should_panic(expected = "closes a combinational loop through an \
                           unregistered channel")]
fn a_ring_of_wires_is_refused() {
    let _ = Ring::lowered("ring");
}

#[test]
fn a_ring_a_register_breaks_is_lowered() {
    let v = HeldRing::verilog("held_ring");
    assert!(v.contains("held_ring_txhdl_chan_u"), "{v}");
}

#[test]
#[should_panic(expected = "was looked at by its receiver before its \
                           sender ran in this step")]
fn a_receiver_joined_before_its_sender_is_refused() {
    let (in_tx, in_rx) = chan::<U<8>, DefaultClock>();
    let (out_tx, _out_rx) = chan::<U<8>, DefaultClock>();
    let mut top = Backwards::default();
    let mut sim = Running::new(top.run(in_rx, out_tx));
    for t in 0..4u8 {
        if in_tx.ready().to_bool() {
            in_tx.send(U::<8>::from(t));
        }
        sim.cycle();
    }
}
