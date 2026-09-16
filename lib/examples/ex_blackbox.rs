// SPDX-License-Identifier: Apache-2.0
//! A foreign module inside a netlist. `blinker` is written by hand, in
//! `blinker.v` and again in `blinker.vhd`, and a lowered unit of units
//! holds it as a child: the netlist instantiates it by its own name,
//! with its parameter, its clock pin and its inout pins joined, and
//! writes no body for it. In the simulation the child is its model in
//! Rust, and the build simulates the netlist with the module's source
//! against the trace that model wrote, so the model and the module are
//! checked against each other cycle by cycle.
use txhdl::comp::trace::{stop, Kind, Wave};
use txhdl::comp::{
    join2, pad, signal, Clock, DefaultClock, In, Out, Pad, Reg, Running, Unit,
};
use txhdl::netlist::{foreign, Lower, Lowered};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

/// The module, as the simulation sees it: what `blinker.v` does, in
/// Rust. The pins are not modelled; nothing in the design reads them.
#[derive(Trace, Default)]
pub struct Blinker<const PERIOD: usize> {
    pub n: Reg<U<8>>,
    pub tick: Reg<U<8>>,
}

impl<const PERIOD: usize> Unit<In<Bit>, (Out<U<8>>, Pad<U<8>>)>
    for Blinker<PERIOD>
{
    async fn run(
        &mut self,
        enable: In<Bit>,
        (count, _pins): (Out<U<8>>, Pad<U<8>>),
    ) {
        loop {
            DefaultClock::rising().await;
            count.set(self.n.get());
            if enable.get().to_bool() {
                if self.tick.get() == U::from((PERIOD - 1) as u8) {
                    self.tick.set(U::from(0u8));
                    self.n.set(self.n.get() + 1);
                } else {
                    self.tick.set(self.tick.get() + 1);
                }
            }
        }
    }
}

/// The module, as the netlist sees it: its name, its ports by their
/// own names in the order `run` takes them, its parameter, and its
/// clock pin.
impl<const PERIOD: usize> Lower for Blinker<PERIOD> {
    fn lowered_as(name: &str) -> Lowered {
        foreign(
            name,
            "blinker",
            &[
                ("enable", Kind::In, 1),
                ("count", Kind::Out, 8),
                ("pins", Kind::Pad, 8),
            ],
            &[("PERIOD", PERIOD as i128)],
            &[("clk", DefaultClock::NAME)],
        )
    }
}

/// A register in front of the enable, as a synchroniser would be: a
/// lowered child beside the foreign one.
#[derive(Trace, Default)]
pub struct Hold {
    pub held: Reg<Bit>,
}

#[lower]
impl Unit for Hold {
    async fn run(&mut self, d: In<Bit>, q: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            with!(self <= { held: d.get() });
            q.set(self.held.get());
        }
    }
}

/// The two, joined. The pins go straight from the top's port to the
/// module's.
#[derive(Trace, Default)]
pub struct Top {
    pub hold: Hold,
    pub blink: Blinker<3>,
}

#[lower]
impl Unit for Top {
    async fn run(
        &mut self,
        enable: In<Bit>,
        (count, pins): (Out<U<8>>, Pad<U<8>>),
    ) {
        let (en_out, en_in) = signal::<Bit, DefaultClock>();
        join2(
            self.hold.run(enable, en_out),
            self.blink.run(en_in, (count, pins)),
        )
        .await;
    }
}

fn main() {
    let (enable_out, enable) = signal::<Bit, DefaultClock>();
    let (count_out, count) = signal::<U<8>, DefaultClock>();
    let mut top = Top::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("enable", &enable);
        w.add("count", &count);
        w.add("top", &top);
        w.start();
    }
    let mut sim = Running::new(
        top.run(enable.clone(), (count_out, pad::<U<8>, DefaultClock>())),
    );
    // Enabled for twenty cycles, off for six, on again.
    println!(" t enable count");
    for t in 0..40 {
        enable_out.set(Bit::from_bool(!(20..26).contains(&t)));
        sim.cycle();
        if t % 4 == 3 {
            println!(
                "{t:2} {:6} {:5}",
                enable.get().to_bool() as u8,
                count.get().raw()
            );
        }
    }
    // The hold costs a cycle, so thirty-three enabled cycles reach the
    // module and the eleventh count lands at the last edge; the output
    // show the count as it stood before that edge, which is ten.
    assert_eq!(count.get().raw(), 10, "the count at the end");
    stop();
    txhdl::netlist::write_vhdl_from_env(&Top::lowered("blackbox"));
    print!("\n{}", Top::verilog("blackbox"));
}
