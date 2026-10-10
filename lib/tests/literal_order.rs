// SPDX-License-Identifier: Apache-2.0
//! A struct literal under `#[lower]` is packed as the struct declares
//! its fields, whatever order the literal names them in (issue 1541).
//! Rust builds the same value from `S { b: x, a: y }` as from
//! `S { a: y, b: x }`, so the two netlists must be the same.
use txhdl::comp::{until, Clock, DefaultClock, Rx, Tx, Unit};
use txhdl::types::U;
use txhdl::{lower, Trace, Transaction, Value};

#[derive(Transaction, Value, Clone, Copy, Default, Debug)]
pub struct S {
    pub a: U<4>,
    pub b: U<8>,
}

#[lower]
fn in_order(x: U<8>, y: U<4>) -> S {
    S { a: y, b: x }
}

#[lower]
fn out_of_order(x: U<8>, y: U<4>) -> S {
    S { b: x, a: y }
}

#[derive(Trace, Default)]
pub struct Declared {}

#[lower]
impl Unit for Declared {
    async fn run(&mut self, inp: Rx<U<8>>, out: Tx<S>) {
        loop {
            until(DefaultClock::rising, || {
                inp.peek().is_some() && out.ready().to_bool()
            })
            .await;
            let x = inp.recv().unwrap_or_default();
            out.send(in_order(x, U::from(5u8)));
        }
    }
}

#[derive(Trace, Default)]
pub struct Swapped {}

#[lower]
impl Unit for Swapped {
    async fn run(&mut self, inp: Rx<U<8>>, out: Tx<S>) {
        loop {
            until(DefaultClock::rising, || {
                inp.peek().is_some() && out.ready().to_bool()
            })
            .await;
            let x = inp.recv().unwrap_or_default();
            out.send(out_of_order(x, U::from(5u8)));
        }
    }
}

#[test]
fn a_literal_out_of_order_lowers_as_one_in_order() {
    let declared = Declared::lowered("m").verilog();
    let swapped = Swapped::lowered("m").verilog();
    assert_eq!(swapped, declared, "the literal's order changed the netlist");
}
