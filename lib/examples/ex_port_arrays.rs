// SPDX-License-Identifier: Apache-2.0
//! Arrays of ports inside a struct of ports (issue 1636): a bus of lanes
//! is one field, `[In<Bit>; 3]`, its ports `side_field_0` onward, read
//! by index in a loop as an array side is; a struct of ports sits beside
//! an array in one tuple side; and an array a child takes may be made of
//! field paths, such as the fields of a split port group.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    join2, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Ports, Trace, Value};

// begin{bus}
/// A result bus of three lanes: which lanes carry a result, and each
/// lane's tag.
#[derive(Ports)]
pub struct Cdb {
    pub valid: [In<Bit>; 3],
    pub tag: [In<U<4>>; 3],
}
// end{bus}

// begin{unit}
/// An entry waiting for the tag `want`: it wakes when any lane carries a
/// result with that tag, and stays awake.
#[derive(Trace, Default)]
pub struct Waiter {
    pub ready: Reg<Bit>,
}

#[lower]
impl Unit for Waiter {
    async fn run(&mut self, (cdb, want): (Cdb, In<U<4>>), woke: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            let mut hit = Bit::Zero;
            for i in 0..3 {
                hit = hit
                    | (cdb.valid[i].get() & (cdb.tag[i].get() == want.get()));
            }
            with!(self <= { hit ? ready: Bit::One });
            woke.set(self.ready.get() | hit);
        }
    }
}
// end{unit}

// begin{units}
/// Two lines as a port group.
#[derive(Value, Clone, Copy, Default, Debug)]
#[value(ports)]
pub struct Pair {
    pub a: Bit,
    pub b: Bit,
}

/// Whether both lines of an array are high.
#[derive(Trace, Default)]
pub struct Both {}

#[lower]
impl Unit for Both {
    async fn run(&mut self, (ins, en): ([In<Bit>; 2], In<Bit>), o: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            o.set(ins[0].get() & ins[1].get() & en.get());
        }
    }
}

/// A unit of units handing its child an array made of a split group's
/// fields.
#[derive(Trace, Default)]
pub struct Gate {
    pub both: Both,
}

#[lower]
impl Unit for Gate {
    async fn run(&mut self, (p, en): (In<Pair>, In<Bit>), o: Out<Bit>) {
        let p = p.split();
        self.both.run(([p.a, p.b], en), o).await;
    }
}
// end{units}

// begin{main}
fn main() {
    let lanes: Vec<_> = (0..3).map(|_| signal::<Bit, DefaultClock>()).collect();
    let tags: Vec<_> = (0..3).map(|_| signal::<U<4>, DefaultClock>()).collect();
    let (want_o, want) = signal::<U<4>, DefaultClock>();
    let (woke_o, woke) = signal::<Bit, DefaultClock>();
    let (pair_o, pair) = signal::<Pair, DefaultClock>();
    let (en_o, en) = signal::<Bit, DefaultClock>();
    let (both_o, both) = signal::<Bit, DefaultClock>();
    let cdb = Cdb {
        valid: [lanes[0].1.clone(), lanes[1].1.clone(), lanes[2].1.clone()],
        tag: [tags[0].1.clone(), tags[1].1.clone(), tags[2].1.clone()],
    };
    let mut waiter = Waiter::default();
    let mut gate = Gate::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("cdb_valid_0", &lanes[0].1);
        w.add("cdb_valid_1", &lanes[1].1);
        w.add("cdb_valid_2", &lanes[2].1);
        w.add("cdb_tag_0", &tags[0].1);
        w.add("cdb_tag_1", &tags[1].1);
        w.add("cdb_tag_2", &tags[2].1);
        w.add("want", &want);
        w.add("waiter", &waiter);
        w.add("woke", &woke);
        w.add("p", &pair);
        w.add("en", &en);
        w.add("o", &both);
        w.start();
    }
    let mut sim = Running::new(join2(
        waiter.run((cdb, want), woke_o),
        gate.run((pair, en), both_o),
    ));
    want_o.set(U::<4>::from(5u8));
    // Each cycle, which lanes carry a result and with which tags.
    let plan: [([bool; 3], [u8; 3]); 6] = [
        ([false, false, false], [0, 0, 0]),
        ([true, false, true], [1, 2, 3]),
        ([false, true, false], [9, 4, 9]),
        ([false, false, true], [0, 0, 5]),
        ([true, true, true], [1, 2, 3]),
        ([false, false, false], [0, 0, 0]),
    ];
    for (c, (v, t)) in plan.iter().enumerate() {
        for k in 0..3 {
            lanes[k].0.set(Bit::from_bool(v[k]));
            tags[k].0.set(U::<4>::from(t[k]));
        }
        pair_o.set(Pair {
            a: Bit::from_bool(c % 2 == 1),
            b: Bit::from_bool(c % 3 != 0),
        });
        en_o.set(Bit::from_bool(c != 4));
        sim.cycle();
        println!(
            "cycle {c}: woke {} both {}",
            woke.get().to_bool(),
            both.get().to_bool()
        );
    }
    stop();
    let (w, g) = (Waiter::lowered("waiter"), Gate::lowered("gate"));
    print!("{}{}", w.verilog(), g.verilog());
    txhdl::netlist::write_netlists_from_env(&[&w, &g]);
}
// end{main}
