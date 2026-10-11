// SPDX-License-Identifier: Apache-2.0
//! Port groups (issue 1647): a struct of plain values marked
//! `#[value(ports)]` is one port of a unit, `In<Ctl>` or `Out<Stat>`, its
//! clock domain written once, and the netlist gives it one port per
//! field. A unit of one loop reads and drives the whole as it would any
//! value; a unit of units splits it and hands each child its field.
//! `#[value(ports, bare)]` keeps the fields' own names as the ports'.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    join2, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace, Value};

// begin{groups}
/// What a panel is told, all on one clock: ports `ctl_en`, `ctl_clear`
/// and `ctl_step` where the port is `ctl`.
#[derive(Value, Clone, Copy, Default, Debug)]
#[value(ports)]
pub struct Ctl {
    pub en: Bit,
    pub clear: Bit,
    pub step: U<4>,
}

/// What it says back.
#[derive(Value, Clone, Copy, Default, Debug)]
#[value(ports)]
pub struct Stat {
    pub count: U<8>,
    pub odd: Bit,
}

/// Two lamps, ports under their own names, `lamp` and `beat`, as a
/// board's pins would be.
#[derive(Value, Clone, Copy, Default, Debug)]
#[value(ports, bare)]
pub struct Leds {
    pub lamp: Bit,
    pub beat: Bit,
}
// end{groups}

// begin{unit}
/// A counter that adds `step` while `en` is high and goes back to nought
/// on `clear`, and says its count and whether it is odd.
#[derive(Trace, Default)]
pub struct Panel {
    pub n: Reg<U<8>>,
}

#[lower]
impl Unit for Panel {
    async fn run(&mut self, ctl: In<Ctl>, stat: Out<Stat>) {
        loop {
            DefaultClock::rising().await;
            let c = ctl.get();
            with!(self <= {
                c.clear ? n: U::<8>::from(0u8),
                (c.en & !c.clear) ? n: self.n + c.step.resize::<8>(),
            });
            let n = self.n.get();
            stat.set(Stat {
                count: n,
                odd: n.bit(0),
            });
        }
    }
}
// end{unit}

// begin{units}
/// A lamp that shows its input a cycle late.
#[derive(Trace, Default)]
pub struct Lamp {
    pub held: Reg<Bit>,
}

#[lower]
impl Unit for Lamp {
    async fn run(&mut self, on: In<Bit>, lit: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            with!(self <= { held: on.get() });
            lit.set(self.held.get());
        }
    }
}

/// Two lamps on the fields of two groups: the panel's enable lights the
/// lamp, its clear the beat.
#[derive(Trace, Default)]
pub struct Desk {
    pub a: Lamp,
    pub b: Lamp,
}

#[lower]
impl Unit for Desk {
    async fn run(&mut self, knobs: In<Ctl>, leds: Out<Leds>) {
        let k = knobs.split();
        let l = leds.split();
        join2(self.a.run(k.en, l.lamp), self.b.run(k.clear, l.beat)).await;
    }
}
// end{units}

// begin{main}
fn main() {
    let (ctl_out, ctl) = signal::<Ctl, DefaultClock>();
    let (stat_out, stat) = signal::<Stat, DefaultClock>();
    let (leds_out, leds) = signal::<Leds, DefaultClock>();
    let knobs = ctl.clone();
    let mut panel = Panel::default();
    let mut desk = Desk::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("ctl", &ctl);
        w.add("knobs", &knobs);
        w.add("panel", &panel);
        w.add("stat", &stat);
        w.add("desk", &desk);
        w.add("leds", &leds);
        w.start();
    }
    let mut sim = Running::new(join2(
        panel.run(ctl, stat_out),
        desk.run(knobs, leds_out),
    ));
    for c in 0..16u8 {
        ctl_out.set(Ctl {
            en: Bit::from_bool(c % 4 != 3),
            clear: Bit::from_bool(c == 10),
            step: U::<4>::from(c % 5 + 1),
        });
        sim.cycle();
        let (s, l) = (stat.get(), leds.get());
        println!(
            "cycle {c:2}: count {:3} odd {} lamp {} beat {}",
            s.count.raw(),
            s.odd.to_bool(),
            l.lamp.to_bool(),
            l.beat.to_bool()
        );
    }
    stop();
    let (p, d) = (Panel::lowered("panel"), Desk::lowered("desk"));
    print!("{}{}", p.verilog(), d.verilog());
    txhdl::netlist::write_netlists_from_env(&[&p, &d]);
}
// end{main}
