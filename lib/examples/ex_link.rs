// SPDX-License-Identifier: Apache-2.0
//! A whole link made in one line.
//!
//! A unit of units makes the channels between its children, and it made
//! them one at a time: `chan::<T, C>()` per channel, so an AXI-Lite link
//! was five lines and five pairs of names, and a board with a dozen
//! links was a hundred and twenty. `link::<B>()` makes a whole bundle
//! (issue 498): `B` is one side's struct of ports, here `LitePort`, and
//! what comes back is the other side's, `LiteHostPort`, and `B`, the
//! same five channels by the same names with each end turned round.
//! Each side is passed on whole, or a channel at a time as `host.aw`.
//!
//! `Relay` forwards each channel of a link from one side to the other,
//! as a register slice does, and `Word` is one word behind a link. The
//! parent takes the link from the testbench, passes it to the relay,
//! and joins the relay to the word with a link it makes whole. The
//! build checks the parent's netlist against the run under nvc and
//! Verilator.
use std::cell::RefCell;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, link, Clock, DefaultClock, Reg, Running, Unit};
use txhdl::types::U;
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::Resp;
use txhdl_parts::bus::axi_lite::{
    axi_lite, LiteAr, LiteAw, LiteB, LiteHost, LiteHostPort, LitePort, LiteR,
    LiteW,
};

// begin{unit}
/// Each channel of a link, from the side it came in on to the other.
#[derive(Trace, Default)]
pub struct Relay {}

#[lower]
impl Unit for Relay {
    async fn run(
        &mut self,
        inp: LitePort<32, 32, 4>,
        out: LiteHostPort<32, 32, 4>,
    ) {
        loop {
            DefaultClock::rising().await;
            let aw = out.aw.ready() & inp.aw.peek().is_some();
            let ar = out.ar.ready() & inp.ar.peek().is_some();
            let w = out.w.ready() & inp.w.peek().is_some();
            let b = inp.b.ready() & out.b.peek().is_some();
            let r = inp.r.ready() & out.r.peek().is_some();
            let awv = inp.aw.recv_if(aw).unwrap_or_default();
            let arv = inp.ar.recv_if(ar).unwrap_or_default();
            let wv = inp.w.recv_if(w).unwrap_or_default();
            let bv = out.b.recv_if(b).unwrap_or_default();
            let rv = out.r.recv_if(r).unwrap_or_default();
            if aw.to_bool() {
                out.aw.send(awv);
            }
            if ar.to_bool() {
                out.ar.send(arv);
            }
            if w.to_bool() {
                out.w.send(wv);
            }
            if b.to_bool() {
                inp.b.send(bv);
            }
            if r.to_bool() {
                inp.r.send(rv);
            }
        }
    }
}

/// One word behind a link: a write stores it, a read returns it.
#[derive(Trace, Default)]
pub struct Word {
    pub word: Reg<U<32>>,
}

#[lower]
impl Unit for Word {
    async fn run(&mut self, bus: LitePort<32, 32, 4>, _o: ()) {
        loop {
            DefaultClock::rising().await;
            let word = self.word.get();
            let wgo = bus.b.ready()
                & bus.aw.peek().is_some()
                & bus.w.peek().is_some();
            let rgo = bus.r.ready() & bus.ar.peek().is_some();
            let wv = bus.w.head();
            let _ = bus.aw.recv_if(wgo);
            let _ = bus.w.recv_if(wgo);
            let _ = bus.ar.recv_if(rgo);
            with!(self <= {
                wgo ? word: wv.data,
            });
            if wgo.to_bool() {
                bus.b.send(LiteB { resp: Resp::Okay });
            }
            if rgo.to_bool() {
                bus.r.send(LiteR {
                    data: word,
                    resp: Resp::Okay,
                });
            }
        }
    }
}

/// The relay and the word, and the link between them made whole.
#[derive(Trace, Default)]
pub struct Slice {
    pub relay: Relay,
    pub word: Word,
}

#[lower]
impl Unit for Slice {
    async fn run(&mut self, bus: LitePort<32, 32, 4>, _o: ()) {
        let (host, per) = link::<LitePort<32, 32, 4>>();
        join2(self.relay.run(bus, host), self.word.run(per, ())).await;
    }
}
// end{unit}

type Host = LiteHost<32, 32, 4>;

/// Send a write's address and word, without waiting.
fn put(h: &Host, data: u32) {
    let (aw, _, w, _, _) = h;
    aw.send(LiteAw {
        addr: U::from(0u8),
        prot: U::from(0u8),
    });
    w.send(LiteW {
        data: U::from(data),
        strb: U::from(0xfu8),
    });
}

/// Wait for a write's response.
async fn done(h: &Host) {
    loop {
        DefaultClock::rising().await;
        if h.3.recv().is_some() {
            return;
        }
    }
}

/// A read, and the word it returned.
async fn get(h: &Host) -> u32 {
    h.1.send(LiteAr {
        addr: U::from(0u8),
        prot: U::from(0u8),
    });
    loop {
        DefaultClock::rising().await;
        if let Some(r) = h.4.recv() {
            return r.data.raw() as u32;
        }
    }
}

fn main() {
    let l = axi_lite::<32, 32, 4>();
    let bus: LitePort<32, 32, 4> = l.per.into();
    let h = l.host;
    let mut slice = Slice::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("bus_aw", &bus.aw);
        w.add("bus_ar", &bus.ar);
        w.add("bus_w", &bus.w);
        w.add("bus_b", &bus.b);
        w.add("bus_r", &bus.r);
        w.add("slice", &slice);
        w.start();
    }
    let seen: Rc<RefCell<Vec<String>>> = Rc::default();
    let log = seen.clone();
    let client = async move {
        put(&h, 0x1234);
        done(&h).await;
        let v = get(&h).await;
        log.borrow_mut().push(format!("wrote 0x1234, read {v:#x}"));
        assert_eq!(v, 0x1234, "the word came back through the relay");
        put(&h, 0xbeef);
        done(&h).await;
        let v = get(&h).await;
        log.borrow_mut().push(format!("wrote 0xbeef, read {v:#x}"));
        assert_eq!(v, 0xbeef);
    };
    let mut sim = Running::new(join2(slice.run(bus, ()), client));
    for _ in 0..40 {
        sim.cycle();
    }
    stop();
    let lines = seen.borrow();
    assert_eq!(lines.len(), 2, "every step finished");
    for s in lines.iter() {
        println!("{s}");
    }
    let net = Slice::lowered("slice");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
