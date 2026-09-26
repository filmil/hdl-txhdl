// SPDX-License-Identifier: Apache-2.0
//! A struct of ports that holds structs of ports.
//!
//! `Links` is two AXI-Lite links, `left` and `right`, each a `LitePort`
//! from `txhdl_parts`. A struct of ports could not hold another: every
//! field had to be one port, so a board that took a host's pins wrote
//! them out one by one and packed them back by hand. Now a field may be
//! a struct of ports itself, and its ports are the netlist's under the
//! field's name, `left_aw_valid`, `right_r_data` (issue 498).
//!
//! `Mailbox` is a unit of units that takes both links as one side and
//! passes each on whole to `Shared`, the word behind two links of
//! `ex_bundle`. `links.left` in its `run` is the nested link, and the
//! netlist joins its five channels to the child's in order. `Links`
//! derives `Ports` as well, so a unit in another crate could take it,
//! and the run checks the ports the derive lists: the two links',
//! flattened, in order.
//!
//! The run is `ex_bundle`'s, and the build checks the netlist of
//! `Mailbox` against it under nvc and Verilator.
use std::cell::RefCell;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, Clock, DefaultClock, Reg, Running, Unit};
use txhdl::netlist::Ports as _;
use txhdl::types::U;
use txhdl::{lower, with, Ports, Trace};
use txhdl_parts::bus::axi::Resp;
use txhdl_parts::bus::axi_lite::{
    axi_lite, LiteAw, LiteB, LiteHost, LitePort, LiteR, LiteW,
};

// begin{unit}
/// One word, written and read from either of two links, which it
/// takes as one side and reads through: `links.left.aw` is the port
/// `left_aw`.
#[derive(Trace, Default)]
pub struct Shared {
    pub word: Reg<U<32>>,
}

#[lower]
impl Unit for Shared {
    async fn run(&mut self, links: Links, _o: ()) {
        loop {
            DefaultClock::rising().await;
            let word = self.word.get();
            let lw = links.left.w.head();
            let rw = links.right.w.head();
            let lwgo = links.left.b.ready()
                & links.left.aw.peek().is_some()
                & links.left.w.peek().is_some();
            let rwgo = links.right.b.ready()
                & links.right.aw.peek().is_some()
                & links.right.w.peek().is_some();
            let lrgo = links.left.r.ready() & links.left.ar.peek().is_some();
            let rrgo = links.right.r.ready() & links.right.ar.peek().is_some();
            let _ = links.left.aw.recv_if(lwgo);
            let _ = links.left.w.recv_if(lwgo);
            let _ = links.right.aw.recv_if(rwgo);
            let _ = links.right.w.recv_if(rwgo);
            let _ = links.left.ar.recv_if(lrgo);
            let _ = links.right.ar.recv_if(rrgo);
            with!(self <= {
                rwgo & !lwgo ? word: rw.data,
                lwgo ? word: lw.data,
            });
            if lwgo.to_bool() {
                links.left.b.send(LiteB { resp: Resp::Okay });
            }
            if rwgo.to_bool() {
                links.right.b.send(LiteB { resp: Resp::Okay });
            }
            if lrgo.to_bool() {
                links.left.r.send(LiteR {
                    data: word,
                    resp: Resp::Okay,
                });
            }
            if rrgo.to_bool() {
                links.right.r.send(LiteR {
                    data: word,
                    resp: Resp::Okay,
                });
            }
        }
    }
}

/// Two links, each a struct of ports from another crate.
#[derive(Ports)]
pub struct Links {
    pub left: LitePort<32, 32, 4>,
    pub right: LitePort<32, 32, 4>,
}

/// The word behind two links, taken as one side and passed on whole:
/// each link a struct of ports nested in it.
#[derive(Trace, Default)]
pub struct Mailbox {
    pub shared: Shared,
}

#[lower]
impl Unit for Mailbox {
    async fn run(&mut self, links: Links, _o: ()) {
        self.shared.run(links, ()).await;
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
    let mut mailbox = Mailbox::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("links_left_aw", &left.aw);
        w.add("links_left_ar", &left.ar);
        w.add("links_left_w", &left.w);
        w.add("links_left_b", &left.b);
        w.add("links_left_r", &left.r);
        w.add("links_right_aw", &right.aw);
        w.add("links_right_ar", &right.ar);
        w.add("links_right_w", &right.w);
        w.add("links_right_b", &right.b);
        w.add("links_right_r", &right.r);
        w.add("mailbox", &mailbox);
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
    let links = Links { left, right };
    let mut sim = Running::new(join2(mailbox.run(links, ()), client));
    for _ in 0..40 {
        sim.cycle();
    }
    stop();
    let lines = seen.borrow();
    assert_eq!(lines.len(), 3, "every step finished");
    for s in lines.iter() {
        println!("{s}");
    }
    // The derive flattens the two links, each port named for its link
    // and its channel, in the order the struct declares them.
    let listed: Vec<String> =
        Links::ports().into_iter().map(|p| p.name).collect();
    assert_eq!(listed.len(), 10, "five channels a link");
    assert_eq!(
        (listed[0].as_str(), listed[9].as_str()),
        ("left_aw", "right_r")
    );
    println!("Links: {}", listed.join(" "));
    let net = Mailbox::lowered("mailbox");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
