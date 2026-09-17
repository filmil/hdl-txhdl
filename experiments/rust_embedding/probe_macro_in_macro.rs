// SPDX-License-Identifier: Apache-2.0
//! Probe 16. Two claims a generated part rests on. One: an attribute
//! macro written by another macro's output expands, so a macro may
//! write `#[lower] impl Unit for ..` and get a lowered unit. Two: a
//! field of a port's value, `p.tag`, reads in a lowered loop as a
//! slice of the port's data, at the bits the value's derive laid out,
//! so a struct payload need not be sliced by hand.
use txhdl::comp::{Clock, DefaultClock, Out, Reg, Rx, Unit};
use txhdl::types::U;
use txhdl::{lower, when, Trace, Transaction, Value};

/// A payload of two fields: the derive lays `tag` above `value`.
#[derive(Transaction, Value, Clone, Copy, Default)]
pub struct Pair {
    pub tag: U<2>,
    pub value: U<8>,
}

/// Writes a unit that keeps the last value whose tag was zero.
macro_rules! keeper {
    ($name:ident) => {
        #[derive(Trace, Default)]
        pub struct $name {
            pub last: Reg<U<8>>,
        }

        #[lower]
        impl Unit for $name {
            async fn run(&mut self, inp: Rx<Pair>, outp: Out<U<8>>) {
                loop {
                    DefaultClock::rising().await;
                    let (offered, p) = inp.take();
                    when!(offered & (p.tag == 0) => self { last: p.value });
                    outp.set(self.last);
                }
            }
        }
    };
}

keeper!(Keeper);

/// The lowered unit exists: the macro's output was expanded.
pub fn verilog() -> String {
    Keeper::verilog("keeper")
}
