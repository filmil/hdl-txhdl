// SPDX-License-Identifier: Apache-2.0
//! An array of ports and a loop over it, in a lowered body (issue 500).
//!
//! `Merge<N>` takes `N` channels and forwards one word a cycle, from the
//! lowest input that has one. Its inputs are one port, `ins: [Rx<U<8>>;
//! N]`, and it goes over them with `for i in 0..N`, carrying what it has
//! found so far from one input to the next in two `let mut`s. `N` is a
//! const parameter the macro cannot see, so the loop is unrolled when
//! `lowered` runs: the netlist of `Merge<3>` has the ports `ins_0` to
//! `ins_2`, and one of `Merge<5>` five. Until now a unit of a count was
//! text a macro wrote once per count, as `router!` and `station!` are.
//!
//! The run offers words on three inputs at once, in bursts, and asserts
//! that they come out lowest input first and none is lost; the netlist
//! is checked against the run under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, mux, Clock, DefaultClock, Reg, Running, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

// begin{unit}
/// `N` channels into one, the lowest-numbered first.
#[derive(Trace, Default)]
pub struct Merge<const N: usize> {
    /// Words forwarded, as a count to watch.
    pub count: Reg<U<8>>,
}

// The lowering reads a loop over an array of ports as `ins[i]`, so the
// index is what it is written with, and Clippy would rather it were an
// iterator.
#[allow(clippy::needless_range_loop)]
#[lower]
impl<const N: usize> Unit for Merge<N> {
    async fn run(&mut self, ins: [Rx<U<8>>; N], out: Tx<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let room = out.ready();
            let mut found = Bit::Zero;
            let mut pick = U::<8>::from(0u8);
            for i in 0..N {
                let mine = room & ins[i].peek().is_some() & !found;
                let _ = ins[i].recv_if(mine);
                pick = mux(mine, ins[i].head(), pick);
                found = found | mine;
            }
            with!(self <= { found ? count: self.count.get() + 1 });
            if found.to_bool() {
                out.send(pick);
            }
        }
    }
}
// end{unit}

fn main() {
    let (t0, r0) = chan::<U<8>, DefaultClock>();
    let (t1, r1) = chan::<U<8>, DefaultClock>();
    let (t2, r2) = chan::<U<8>, DefaultClock>();
    let (out_tx, out_rx) = chan::<U<8>, DefaultClock>();
    let mut unit = Merge::<3>::default();
    let ins = [r0, r1, r2];
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("ins_0", &ins[0]);
        w.add("ins_1", &ins[1]);
        w.add("ins_2", &ins[2]);
        w.add("merge", &unit);
        w.add("out", &out_rx);
        w.start();
    }
    let senders = [t0, t1, t2];
    let mut sim = Running::new(unit.run(ins, out_tx));
    // Each input has four words, 10 * input + k, offered from cycle
    // 2 * input on, so all three are waiting at once for a while.
    let mut next = [0u8; 3];
    let mut got: Vec<u8> = Vec::new();
    for cycle in 0..40u8 {
        for (i, tx) in senders.iter().enumerate() {
            if cycle >= 2 * i as u8 && next[i] < 4 && tx.ready().to_bool() {
                tx.send(U::from(10 * i as u8 + next[i]));
                next[i] += 1;
            }
        }
        // The consumer pauses now and then.
        if cycle % 7 != 3 {
            if let Some(v) = out_rx.recv() {
                got.push(v.raw() as u8);
            }
        }
        sim.cycle();
    }
    println!("forwarded {} words: {got:?}", got.len());
    assert_eq!(got.len(), 12, "every word came out");
    for i in 0..3u8 {
        let mine: Vec<u8> =
            got.iter().copied().filter(|v| v / 10 == i).collect();
        assert_eq!(
            mine,
            vec![10 * i, 10 * i + 1, 10 * i + 2, 10 * i + 3],
            "input {i} in order"
        );
    }
    stop();
    let net = Merge::<3>::lowered("merge");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
    // The same source at another count: five inputs, five ports.
    let five = Merge::<5>::lowered("merge5");
    let ports: Vec<&str> = five.ports.iter().map(|p| p.0.as_str()).collect();
    println!("\nMerge<5>'s ports: {ports:?}");
}
