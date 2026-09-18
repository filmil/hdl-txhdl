// SPDX-License-Identifier: Apache-2.0
//! Probe 26. Expected to fail: a tuple `let` in a lowered function
//! whose value is not a tuple of the same length. A helper's `let` is
//! a substitution, so the lowering pairs the names with the values and
//! has nothing to pair here; it says so where the helper is called
//! rather than writing a netlist that names `x` and `y` with nothing
//! declaring them, which is what issue 159 was.
#![allow(clippy::all)]
use txhdl::comp::{Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::U;
use txhdl::{lower, Trace};

/// Plain Rust, and not lowered: the lowering sees a call, not a tuple.
fn halves(a: U<8>) -> (U<8>, U<8>) {
    (a, a)
}

#[lower]
fn pair(a: U<8>) -> U<8> {
    let (x, y) = halves(a);
    x ^ y
}

#[derive(Trace, Default)]
pub struct Pairs {
    pub last: Reg<U<8>>,
}

#[lower]
impl Unit for Pairs {
    async fn run(&mut self, (a,): (In<U<8>>,), (q,): (Out<U<8>>,)) {
        loop {
            DefaultClock::rising().await;
            self.last.set(pair(a.get()));
            q.set(self.last);
        }
    }
}
