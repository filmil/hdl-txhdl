// SPDX-License-Identifier: Apache-2.0
//! Two lowered units that deadlock when a Verilog top joins their
//! channel wire to wire (#1868), and the same two joined by TxHDL's own
//! channel, which does not.
//!
//! `Count` offers its count with `put`, whose valid waits on ready, as
//! `EthRx`'s does. `Stage` takes a word only when one is offered and it
//! has room to pass it on, so its ready waits on valid. Joined directly
//! neither rises first. `Pair` joins them by `chan()`, which lowers to
//! the two-entry `pair_txhdl_chan`, whose ready is a register.
//!
//! `PairU` is the same pair over an unregistered channel; its netlist
//! holds both kinds of channel buffer, which `offer_tb` drives with a
//! sender that does not wait for ready (#1871).
//!
//! Run with five paths: it writes `count.v`, `stage.v`, `pair.v`,
//! `pair_u.v` and `pair_u.vhd`.
use txhdl::comp::{
    chan, join2, Clock, DefaultClock, In, Out, Reg, Rx, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

/// Offers its count with `put`, and counts on once it is taken.
#[derive(Trace, Default)]
pub struct Count {
    pub n: Reg<U<8>>,
}

#[lower]
impl Unit<(), Tx<U<8>>> for Count {
    async fn run(&mut self, _i: (), out: Tx<U<8>>) {
        loop {
            out.put(|| self.n.get()).await;
            self.n.set(self.n + 1);
        }
    }
}

/// Passes a word on in the cycle it is offered, when there is room.
#[derive(Trace, Default)]
pub struct Stage {}

#[lower]
impl Unit<Rx<U<8>>, Tx<U<8>>> for Stage {
    async fn run(&mut self, inp: Rx<U<8>>, out: Tx<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let go = Bit::from_bool(inp.peek().is_some()) & out.ready();
            let v = inp.head();
            let _ = inp.recv_if(go);
            if go.to_bool() {
                out.send(v);
            }
        }
    }
}

/// The two joined by TxHDL's own channel.
#[derive(Trace, Default)]
pub struct Pair {
    pub count: Count,
    pub stage: Stage,
}

#[lower]
impl Unit<(), Tx<U<8>>> for Pair {
    async fn run(&mut self, _i: (), out: Tx<U<8>>) {
        let (tx, rx) = chan::<U<8>, DefaultClock>();
        join2(self.count.run((), tx), self.stage.run(rx, out)).await;
    }
}

/// The same two joined by an unregistered channel, which lowers to
/// `pair_u_txhdl_chan_u`. Its netlist is here for its two buffers, the
/// registered and the unregistered, which `offer_tb` drives on their
/// own (#1871).
#[derive(Trace, Default)]
pub struct PairU {
    pub count: Count,
    pub stage: Stage,
}

#[lower]
impl Unit<(), Tx<U<8>>> for PairU {
    async fn run(&mut self, _i: (), out: Tx<U<8>>) {
        #[unregistered]
        let (tx, rx) = chan::<U<8>, DefaultClock>();
        join2(self.count.run((), tx), self.stage.run(rx, out)).await;
    }
}

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    let [count, stage, pair, pair_u, pair_u_vhd] = paths.as_slice() else {
        panic!("usage: gen COUNT.v STAGE.v PAIR.v PAIR_U.v PAIR_U.vhd");
    };
    let w = |p: &str, s: String| std::fs::write(p, s).unwrap();
    w(count, Count::lowered("count").verilog());
    w(stage, Stage::lowered("stage").verilog());
    w(pair, Pair::lowered("pair").verilog());
    w(pair_u, PairU::lowered("pair_u").verilog());
    w(pair_u_vhd, PairU::lowered("pair_u").vhdl());
}

/// No channel at all: a wire in and a wire out.
#[derive(Trace, Default)]
pub struct Plain {}

#[lower]
impl Unit<In<U<8>>, Out<U<8>>> for Plain {
    async fn run(&mut self, inp: In<U<8>>, out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            out.set(inp.get());
        }
    }
}

/// Takes every word: its only channel port is a receiving one.
#[derive(Trace, Default)]
pub struct Sink {}

#[lower]
impl Unit<Rx<U<8>>, ()> for Sink {
    async fn run(&mut self, inp: Rx<U<8>>, _o: ()) {
        loop {
            DefaultClock::rising().await;
            let _ = inp.recv_if(Bit::One);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENDS: &str = "// txhdl: a sending port's valid here waits";
    const TAKES: &str = "// txhdl: a receiving port's ready here waits";
    const JOIN: &str = "// txhdl: join through <top>_txhdl_chan";

    /// The header of module `name` in `v`: what comes before its line.
    fn header<'a>(v: &'a str, name: &str) -> &'a str {
        let at = v.find(&format!("module {name}(")).expect("the module");
        let start = v[..at].rfind("`timescale").unwrap_or(0);
        &v[start..at]
    }

    /// Each module says what its channel ports wait on, the first thing
    /// whoever writes a top reads, and one with none says nothing
    /// (#1868).
    #[test]
    fn each_module_says_what_its_channel_ports_wait_on() {
        let v = Count::lowered("count").verilog();
        let h = header(&v, "count");
        assert!(h.contains(SENDS) && !h.contains(TAKES), "{h}");
        assert!(h.contains(JOIN), "{h}");
        let v = Sink::lowered("sink").verilog();
        let h = header(&v, "sink");
        assert!(h.contains(TAKES) && !h.contains(SENDS), "{h}");
        assert!(h.contains(JOIN), "{h}");
        let v = Stage::lowered("stage").verilog();
        let h = header(&v, "stage");
        assert!(h.contains(SENDS) && h.contains(TAKES), "{h}");
        let v = Plain::lowered("plain").verilog();
        assert!(!v.contains("txhdl:"), "{v}");
    }

    /// A unit of units' file holds its children's modules, and each
    /// says it too: `pair` sends, and its children are a sender and a
    /// stage.
    #[test]
    fn a_child_module_in_a_parents_file_says_it_too() {
        let v = Pair::lowered("pair").verilog();
        let h = header(&v, "pair");
        assert!(h.contains(SENDS) && !h.contains(TAKES), "{h}");
        let children: Vec<&str> = v
            .lines()
            .filter_map(|l| l.strip_prefix("module "))
            .filter_map(|l| l.split('(').next())
            .filter(|m| *m != "pair" && !m.contains("txhdl_chan"))
            .collect();
        assert_eq!(children.len(), 2, "{children:?}");
        for m in children {
            assert!(header(&v, m).contains(SENDS), "{m}");
        }
    }
}
