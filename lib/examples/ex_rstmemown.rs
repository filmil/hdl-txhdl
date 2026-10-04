// SPDX-License-Identifier: Apache-2.0
//! A unit that answers the reset itself writes its memory under reset.
//!
//! A reset drops a memory write made while it is held, as it drops a
//! register's drive (issue 877). A unit that declares `rst` is the
//! exception for its registers: its body says what the reset does, and
//! the run leaves them to it (issue 878). Its memory is the same, and
//! the clearest reason is the one here: a store that wipes itself while
//! the reset is held, a word a cycle, so that it comes out of reset
//! empty whatever it held before (issue 966).
//!
//! The run fills the store, holds the reset for as many cycles as the
//! store has words, and reads it back: every word is the one the wipe
//! wrote, and a write made after the reset lands as usual. The run and
//! the netlist are checked against each other under nvc and Verilator,
//! through the read port, since a memory is not traced.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    mux, set_reset, signal, Clock, DefaultClock, In, Mem, Out, Reg, Running,
    Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

/// What the wipe writes.
const WIPED: u8 = 0xee;

// begin{unit}
/// A store of four words that wipes itself under reset.
#[derive(Trace, Default)]
pub struct Wipe {
    /// The words.
    pub m: Mem<U<8>, 4>,
    /// The word the wipe writes next.
    pub at: Reg<U<2>>,
}

#[lower]
impl Unit for Wipe {
    async fn run(
        &mut self,
        (rst, we, d, a): (In<Bit>, In<Bit>, In<U<8>>, In<U<2>>),
        q: Out<U<8>>,
    ) {
        loop {
            DefaultClock::rising().await;
            // Under reset a word a cycle is wiped, at `at`; out of it
            // the word given is written at the address given.
            let r = rst.get();
            let at = self.at.get();
            let wipe = U::<8>::from(WIPED);
            let slot = mux(r, at, a.get());
            let word = mux(r, wipe, d.get());
            with!(self <= {
                r | we.get() ? m.at(slot): word,
                r ? at: at + 1,
            });
            q.set(self.m.read(a.get()));
        }
    }
}
// end{unit}

fn main() {
    let (rst_o, rst) = signal::<Bit, DefaultClock>();
    let (we_o, we) = signal::<Bit, DefaultClock>();
    let (d_o, d) = signal::<U<8>, DefaultClock>();
    let (a_o, a) = signal::<U<2>, DefaultClock>();
    let (q_o, q) = signal::<U<8>, DefaultClock>();
    let mut store = Wipe::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("rst", &rst);
        w.add("we", &we);
        w.add("d", &d);
        w.add("a", &a);
        w.add("q", &q);
        w.add("wipe", &store);
        w.start();
    }
    let mut sim = Running::new(store.run((rst, we, d, a), q_o));

    // Every word filled.
    rst_o.set(Bit::Zero);
    we_o.set(Bit::One);
    for w in 0..4u8 {
        a_o.set(U::<2>::from(w));
        d_o.set(U::<8>::from(0x10 + w));
        sim.cycle();
    }
    // The reset held for four cycles: the wipe.
    we_o.set(Bit::Zero);
    set_reset(true);
    rst_o.set(Bit::One);
    for _ in 0..4 {
        sim.cycle();
    }
    set_reset(false);
    rst_o.set(Bit::Zero);
    // One write after it, at word 2.
    we_o.set(Bit::One);
    a_o.set(U::<2>::from(2u8));
    d_o.set(U::<8>::from(0x42u8));
    sim.cycle();
    we_o.set(Bit::Zero);

    // The store read back, a word a cycle.
    let mut words = Vec::new();
    for w in 0..4u8 {
        a_o.set(U::<2>::from(w));
        sim.cycle();
        words.push(q.get().raw() as u8);
    }
    println!("after the wipe and one write: {words:02x?}");
    stop();
    assert_eq!(
        words,
        [WIPED, WIPED, 0x42, WIPED],
        "the wipe under reset, then the write after it"
    );
    let net = Wipe::lowered("wipe");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
