// SPDX-License-Identifier: Apache-2.0
//! What an empty channel's head reads: zero, in the run and in the
//! netlist alike (issue 1324). `Rx::head` on an empty channel is the
//! type's default, and a unit that looks at it every cycle, offered or
//! not, sees that zero between transactions. The netlist's channel had
//! the head register's last contents there instead, so a unit that
//! read an empty head ran one way in Rust and another in hardware; the
//! channel now drives its data to zero while nothing is valid.
//!
//! A source sends a count every fourth cycle, and a looker drives its
//! output with the channel's head every cycle, taking whatever is
//! offered. `Pair` joins them by a registered channel and `PairU` by an
//! unregistered one, and both netlists are checked against this run
//! under both simulators, the empty cycles included.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, signal, Clock, DefaultClock, Out, Reg, Running, Rx, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

// begin{units}
/// A count, sent every fourth cycle while the channel has room.
#[derive(Trace, Default)]
pub struct Src {
    pub at: Reg<U<2>>,
    pub n: Reg<U<8>>,
}

#[lower]
impl Unit for Src {
    async fn run(&mut self, _i: (), out: Tx<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let go = Bit::from(self.at.get() == 0) & out.ready();
            if go.to_bool() {
                out.send(self.n.get() + 1);
            }
            with!(self <= {
                at: self.at.get() + 1,
                go ? n: self.n.get() + 1,
            });
        }
    }
}

/// The channel's head on its output every cycle, offered or not, and
/// whatever is offered taken.
#[derive(Trace, Default)]
pub struct Look {}

#[lower]
impl Unit for Look {
    async fn run(&mut self, inp: Rx<U<8>>, look: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            look.set(inp.head());
            let _ = inp.recv_if(Bit::One);
        }
    }
}
// end{units}

// begin{pairs}
/// The two joined by a registered channel.
#[derive(Trace, Default)]
pub struct Pair {
    pub src: Src,
    pub look: Look,
}

#[lower]
impl Unit for Pair {
    async fn run(&mut self, _i: (), seen: Out<U<8>>) {
        let (tx, rx) = chan::<U<8>, DefaultClock>();
        join2(self.src.run((), tx), self.look.run(rx, seen)).await;
    }
}

/// The same joined by an unregistered channel, the sender first.
#[derive(Trace, Default)]
pub struct PairU {
    pub src: Src,
    pub look: Look,
}

#[lower]
impl Unit for PairU {
    async fn run(&mut self, _i: (), seen_u: Out<U<8>>) {
        #[unregistered]
        let (tx, rx) = chan::<U<8>, DefaultClock>();
        join2(self.src.run((), tx), self.look.run(rx, seen_u)).await;
    }
}
// end{pairs}

fn main() {
    let (seen_o, seen) = signal::<U<8>, DefaultClock>();
    let (seenu_o, seen_u) = signal::<U<8>, DefaultClock>();
    let mut pair = Pair::default();
    let mut pairu = PairU::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("seen", &seen);
        w.add("seen_u", &seen_u);
        w.add("pair", &pair);
        w.add("pairu", &pairu);
        w.start();
    }
    let mut sim =
        Running::new(join2(pair.run((), seen_o), pairu.run((), seenu_o)));
    println!(" t  registered  unregistered");
    let (mut zeros, mut counts) = (0, 0);
    for t in 0..16u32 {
        sim.cycle();
        let (a, b) = (seen.get().raw(), seen_u.get().raw());
        println!("{t:2}  {a:10}  {b:12}");
        for v in [a, b] {
            if v == 0 {
                zeros += 1;
            } else {
                counts += 1;
            }
        }
    }
    // Most cycles find the channel empty, and read zero there.
    assert!(zeros > counts, "an empty head reads zero");
    println!("\n{counts} looks saw a count, {zeros} found the channel empty");
    stop();
    txhdl::netlist::write_netlists_from_env(&[
        &Pair::lowered("empty_pair"),
        &PairU::lowered("empty_pair_u"),
    ]);
}
