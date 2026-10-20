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
//! Run with three paths: it writes `count.v`, `stage.v` and `pair.v`.
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

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    let [count, stage, pair] = paths.as_slice() else {
        panic!("usage: gen COUNT.v STAGE.v PAIR.v");
    };
    std::fs::write(count, Count::lowered("count").verilog()).unwrap();
    std::fs::write(stage, Stage::lowered("stage").verilog()).unwrap();
    std::fs::write(pair, Pair::lowered("pair").verilog()).unwrap();
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

#[cfg(test)]
mod tests {
    use super::*;

    const RULE: &str = "// txhdl: a channel port here waits on the other side";

    /// Each module with a channel port says the rule in its header, the
    /// first thing whoever writes a top reads; one without says
    /// nothing (#1868).
    #[test]
    fn the_rule_is_in_the_header_of_a_module_with_a_channel() {
        for (name, v) in [
            ("count", Count::lowered("count").verilog()),
            ("stage", Stage::lowered("stage").verilog()),
            ("pair", Pair::lowered("pair").verilog()),
        ] {
            let header = &v[..v.find(&format!("module {name}(")).unwrap()];
            assert!(header.contains(RULE), "{name}:\n{header}");
            assert!(header.contains("<top>_txhdl_chan"), "{name}");
        }
        let v = Plain::lowered("plain").verilog();
        assert!(!v.contains(RULE), "{v}");
    }
}
