// SPDX-License-Identifier: Apache-2.0
//! A struct of ports from another crate, taken twice by one unit.
//!
//! `LitePort` is declared in `txhdl_parts`, beside the AXI-Lite beats,
//! with `#[derive(Ports)]`: the five channels a peripheral holds on a
//! link. This crate cannot see its source, and does not need to. The
//! derive is all `#[lower]` asks of a struct declared elsewhere, so any
//! unit anywhere takes a whole link as one port and reads `left.ar`.
//!
//! `Shared` is one word behind two links, `left` and `right`, as a
//! mailbox between two processors would be. Both sides are the same
//! type, and the netlist keeps them apart by naming each port for its
//! side as well as its field: `left_aw_valid`, `right_r_data`. A write
//! from both sides in one cycle keeps the left one's word.
//!
//! The run writes from one side and reads from the other, both ways,
//! then writes from both at once. The netlist is checked against this
//! run under nvc and Verilator.
use std::cell::RefCell;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, Clock, DefaultClock, Reg, Running, Unit};
use txhdl::types::U;
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::Resp;
use txhdl_parts::bus::axi_lite::{
    axi_lite, LiteAw, LiteB, LiteHost, LitePort, LiteR, LiteW,
};

// begin{unit}
/// One word, written and read from either of two links.
#[derive(Trace, Default)]
pub struct Shared {
    pub word: Reg<U<32>>,
}

#[lower]
impl Unit for Shared {
    async fn run(
        &mut self,
        left: LitePort<32, 32, 4>,
        right: LitePort<32, 32, 4>,
    ) {
        loop {
            DefaultClock::rising().await;
            let word = self.word.get();
            let lw = left.w.head();
            let rw = right.w.head();
            let lwgo = left.b.ready()
                & left.aw.peek().is_some()
                & left.w.peek().is_some();
            let rwgo = right.b.ready()
                & right.aw.peek().is_some()
                & right.w.peek().is_some();
            let lrgo = left.r.ready() & left.ar.peek().is_some();
            let rrgo = right.r.ready() & right.ar.peek().is_some();
            let _ = left.aw.recv_if(lwgo);
            let _ = left.w.recv_if(lwgo);
            let _ = right.aw.recv_if(rwgo);
            let _ = right.w.recv_if(rwgo);
            let _ = left.ar.recv_if(lrgo);
            let _ = right.ar.recv_if(rrgo);
            with!(self <= {
                rwgo & !lwgo ? word: rw.data,
                lwgo ? word: lw.data,
            });
            if lwgo.to_bool() {
                left.b.send(LiteB { resp: Resp::Okay });
            }
            if rwgo.to_bool() {
                right.b.send(LiteB { resp: Resp::Okay });
            }
            if lrgo.to_bool() {
                left.r.send(LiteR {
                    data: word,
                    resp: Resp::Okay,
                });
            }
            if rrgo.to_bool() {
                right.r.send(LiteR {
                    data: word,
                    resp: Resp::Okay,
                });
            }
        }
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
    h.1.send(LiteAw {
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
    let r = axi_lite::<32, 32, 4>();
    let left: LitePort<32, 32, 4> = l.per.into();
    let right: LitePort<32, 32, 4> = r.per.into();
    let (lh, rh) = (l.host, r.host);
    let mut shared = Shared::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("left_aw", &left.aw);
        w.add("left_ar", &left.ar);
        w.add("left_w", &left.w);
        w.add("left_b", &left.b);
        w.add("left_r", &left.r);
        w.add("right_aw", &right.aw);
        w.add("right_ar", &right.ar);
        w.add("right_w", &right.w);
        w.add("right_b", &right.b);
        w.add("right_r", &right.r);
        w.add("shared", &shared);
        w.start();
    }
    let seen: Rc<RefCell<Vec<String>>> = Rc::default();
    let log = seen.clone();
    let client = async move {
        let say = |s: String| log.borrow_mut().push(s);
        put(&lh, 0x1234);
        done(&lh).await;
        let v = get(&rh).await;
        say(format!("left wrote 0x1234, right read {v:#x}"));
        assert_eq!(v, 0x1234);
        put(&rh, 0xbeef);
        done(&rh).await;
        let v = get(&lh).await;
        say(format!("right wrote 0xbeef, left read {v:#x}"));
        assert_eq!(v, 0xbeef);
        put(&lh, 0xaaaa);
        put(&rh, 0x5555);
        join2(done(&lh), done(&rh)).await;
        let a = get(&lh).await;
        let b = get(&rh).await;
        say(format!(
            "both wrote in one cycle, left read {a:#x}, right read {b:#x}"
        ));
        assert_eq!((a, b), (0xaaaa, 0xaaaa), "the left write wins");
    };
    let mut sim = Running::new(join2(shared.run(left, right), client));
    for _ in 0..40 {
        sim.cycle();
    }
    stop();
    let lines = seen.borrow();
    assert_eq!(lines.len(), 3, "every step finished");
    for s in lines.iter() {
        println!("{s}");
    }
    // Not `shared`, which VHDL reserves (issue 485).
    let net = Shared::lowered("mailbox");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
