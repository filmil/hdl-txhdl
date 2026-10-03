// SPDX-License-Identifier: Apache-2.0
//! A reset leaves a memory's words alone and drops a write.
//!
//! A reset puts every register back and empties every channel, and a
//! memory is neither: it keeps what it holds, since a reset is not a
//! reload. What a reset does to a memory is drop a write made while it
//! is held, as it drops a register's drive, so that a unit held in
//! reset changes nothing (issue 877).
//!
//! The log below has two stores and a process for each. The first
//! writes its store where a register, the write pointer, says, as the
//! write side of a FIFO does; the second writes its store alone, at an
//! address it is given. The run writes a word into each, holds the
//! reset over a second write, and reads both stores back: the first
//! word is there in each, and the second in neither. The run and the
//! netlist are checked against each other under nvc and Verilator,
//! through the read ports, since a memory is not traced.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    join2, set_reset, signal, Clock, DefaultClock, In, Mem, Out, Reg, Running,
    Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

// begin{unit}
/// Two stores: one written at a pointer, one at an address given.
#[derive(Trace, Default)]
pub struct Log {
    /// Written where `wp` says, a word a write.
    pub m: Mem<U<8>, 4>,
    /// Where the next word goes in `m`.
    pub wp: Reg<U<2>>,
    /// Written at the address given, by a process with no register.
    pub n: Mem<U<8>, 4>,
}

#[lower]
impl Unit for Log {
    async fn run(
        &mut self,
        (we, d, a): (In<Bit>, In<U<8>>, In<U<2>>),
        (q, r): (Out<U<8>>, Out<U<8>>),
    ) {
        join2(
            async {
                loop {
                    DefaultClock::rising().await;
                    let wp = self.wp.get();
                    when!(we.get() => self { m.at(wp): d.get(), wp: wp + 1 });
                    q.set(self.m.read(a.get()));
                }
            },
            async {
                loop {
                    DefaultClock::rising().await;
                    when!(we.get() => self { n.at(a.get()): d.get() });
                    r.set(self.n.read(a.get()));
                }
            },
        )
        .await;
    }
}
// end{unit}

fn main() {
    let (we_o, we) = signal::<Bit, DefaultClock>();
    let (d_o, d) = signal::<U<8>, DefaultClock>();
    let (a_o, a) = signal::<U<2>, DefaultClock>();
    let (q_o, q) = signal::<U<8>, DefaultClock>();
    let (r_o, r) = signal::<U<8>, DefaultClock>();
    let mut log = Log::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("we", &we);
        w.add("d", &d);
        w.add("a", &a);
        w.add("q", &q);
        w.add("r", &r);
        w.add("log", &log);
        w.start();
    }
    let mut sim = Running::new(log.run((we, d, a), (q_o, r_o)));

    // A word into each store, at word 0.
    we_o.set(Bit::One);
    d_o.set(U::<8>::from(0x11u8));
    a_o.set(U::<2>::from(0u8));
    sim.cycle();
    // A second, with the reset held: at word 1 of each.
    set_reset(true);
    d_o.set(U::<8>::from(0x77u8));
    a_o.set(U::<2>::from(1u8));
    sim.cycle();
    set_reset(false);
    we_o.set(Bit::Zero);

    // Both stores read back, a word a cycle.
    let mut first = Vec::new();
    let mut second = Vec::new();
    for w in 0..4u8 {
        a_o.set(U::<2>::from(w));
        sim.cycle();
        first.push(q.get().raw() as u8);
        second.push(r.get().raw() as u8);
    }
    println!("at the pointer: {first:02x?}");
    println!("at the address: {second:02x?}");
    stop();
    assert_eq!(first, [0x11, 0, 0, 0], "the pointer's write in reset");
    assert_eq!(second, [0x11, 0, 0, 0], "the address's write in reset");
    let net = Log::lowered("log");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
