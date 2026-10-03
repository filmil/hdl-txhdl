// SPDX-License-Identifier: Apache-2.0
//! A unit that answers the reset itself, and the reset that leaves it
//! to.
//!
//! A unit that only holds registers needs to know nothing about the
//! reset: while it is high, its registers go back to where they
//! started. A unit that wants to do something under reset, rather
//! than merely forget, takes `rst: In<Bit>` and reads it, and then
//! what it does under reset is what its body says and nothing else.
//! The netlist gives it no clearing branch of its own, and the run
//! does not put its registers back on top of what the body drives
//! (issue 878). A core is the case in point: under reset it goes to
//! its reset vector, not to zero.
//!
//! The fetch below counts its program counter up, and under reset
//! loads the vector `0x80`. The run counts three, holds the reset for
//! an edge, and counts on from the vector. The port and the reset the
//! netlist gives every module are one net, so the run drives both at
//! once, and the netlist is checked against the run under nvc and
//! Verilator through the reset and out of it.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    mux, set_reset, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

// begin{unit}
/// A program counter that goes to its reset vector under reset.
#[derive(Trace, Default)]
pub struct Fetch {
    /// The address of the next fetch.
    pub pc: Reg<U<8>>,
}

#[lower]
impl Unit for Fetch {
    async fn run(&mut self, rst: In<Bit>, pc_out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            self.pc
                .set(mux(rst.get(), U::<8>::from(0x80u8), self.pc + 1));
            pc_out.set(self.pc);
        }
    }
}
// end{unit}

fn main() {
    let (rst_o, rst) = signal::<Bit, DefaultClock>();
    let (pc_o, pc) = signal::<U<8>, DefaultClock>();
    let mut fetch = Fetch::default();
    let at = fetch.pc;
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("rst", &rst);
        w.add("pc_out", &pc);
        w.add("fetch", &fetch);
        w.start();
    }
    let mut sim = Running::new(fetch.run(rst, pc_o));
    // The reset and the unit's own port, one net in the netlist.
    let hold = |on: bool| {
        set_reset(on);
        rst_o.set(Bit::from_bool(on));
    };

    hold(false);
    for _ in 0..3 {
        sim.cycle();
    }
    let counted = at.get().raw() as u8;
    println!("after 3 edges:           pc = {counted:#04x}");
    hold(true);
    sim.cycle();
    let vector = at.get().raw() as u8;
    println!("after 1 edge in reset:   pc = {vector:#04x}");
    hold(false);
    sim.cycle();
    let on = at.get().raw() as u8;
    println!("after 1 edge out of it:  pc = {on:#04x}");
    stop();
    assert_eq!(
        (counted, vector, on),
        (0x03, 0x80, 0x81),
        "the reset vector, not zero"
    );
    let net = Fetch::lowered("fetch");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
