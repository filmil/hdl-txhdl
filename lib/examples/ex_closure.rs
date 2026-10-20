// SPDX-License-Identifier: Apache-2.0
//! A closure in a lowered body, inlined at each call as a function
//! under `#[lower]` would be, and a take with `rx.recv_if(c);` as a
//! statement (issue 1838). A byte-serial header check: a counter sends
//! bytes, and a checker takes them while `go` is high, counts their
//! positions, and flags a byte at position 12 or 13 that is not the one
//! expected there. Two closures say the check: `at`, whether the byte
//! is at a position, which reads the body's `p`, and `ne`, whether two
//! bytes differ. Checked under nvc and Verilator against its trace.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, now, signal, Clock, DefaultClock, In, Out, Reg, Running, Rx,
    Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

// begin{unit}
/// Sends 0, 1, 2 and on, one a cycle when the channel takes it: the
/// byte at each position is the position's number.
#[derive(Trace, Default)]
pub struct Bytes {
    pub n: Reg<U<8>>,
}

#[lower]
impl Unit for Bytes {
    async fn run(&mut self, _i: (), out: Tx<U<8>>) {
        loop {
            DefaultClock::rising().await;
            if out.ready().to_bool() {
                out.send(self.n);
                self.n.set(self.n + 1);
            }
        }
    }
}

/// Takes a byte while `go` is high, counts its position, and keeps
/// `bad` once a header byte was not the one expected.
#[derive(Trace, Default)]
pub struct Check {
    pub pos: Reg<U<11>>,
    pub bad: Reg<Bit>,
}

#[lower]
impl Unit for Check {
    async fn run(&mut self, (inp, go): (Rx<U<8>>, In<Bit>), bad_o: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            let p = self.pos.get();
            let d = inp.head();
            let take = go.get() & inp.peek().is_some();
            // Only the take matters here, so nothing is bound.
            inp.recv_if(take);
            let at = |k: U<11>| Bit::from(p == k);
            let ne = |a: U<8>, b: U<8>| Bit::from(a != b);
            let miss = (at(U::<11>::from(12u16)) & ne(d, U::<8>::from(12u8)))
                | (at(U::<11>::from(13u16)) & ne(d, U::<8>::from(0x00u8)));
            when!(take => self { pos: p + U::<11>::from(1u16) });
            when!(take & miss => self { bad: Bit::One });
            bad_o.set(self.bad.get());
        }
    }
}

/// The two, and the channel between them.
#[derive(Trace, Default)]
pub struct Pair {
    pub bytes: Bytes,
    pub check: Check,
}

#[lower]
impl Unit for Pair {
    async fn run(&mut self, go: In<Bit>, bad: Out<Bit>) {
        let (tx, rx) = chan::<U<8>, DefaultClock>();
        join2(self.bytes.run((), tx), self.check.run((rx, go), bad)).await;
    }
}
// end{unit}

fn main() {
    let (go_out, go) = signal::<Bit, DefaultClock>();
    let (bad_out, bad) = signal::<Bit, DefaultClock>();
    let mut pair = Pair::default();
    let pos = pair.check.pos;
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("go", &go);
        w.add("bad", &bad);
        w.add("pair", &pair);
        w.start();
    }
    let mut sim = Running::new(pair.run(go, bad_out));
    // Take on all but every fourth cycle, until past position 13.
    let mut flagged_at = None;
    for c in 0..24 {
        go_out.set(c % 4 != 3);
        sim.cycle();
        let p = pos.get().raw();
        let b = bad.get().to_bool();
        println!("t={:>2} pos {:>2} bad {}", now(), p, b as u8);
        if b && flagged_at.is_none() {
            flagged_at = Some(p);
        }
    }
    stop();
    // Position 12 holds 12, as expected; position 13 holds 13, not 0,
    // so `bad` rises once that byte is taken.
    assert_eq!(flagged_at, Some(14), "bad after the byte at 13");
    let net = Pair::lowered("hdr");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
