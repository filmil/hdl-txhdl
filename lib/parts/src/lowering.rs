// SPDX-License-Identifier: Apache-2.0
//! Units the lowering once refused and must accept, kept here so that
//! the refusal cannot come back: the test crate stops compiling if it
//! does.

use txhdl::comp::{Clock, DefaultClock, Reg, Tx, Unit};
use txhdl::types::U;
use txhdl::{lower, Trace};
use txhdl::{Transaction as TransactionDerive, Value as ValueDerive};

/// A word with a field named as the local below.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct Word {
    pub addr: U<8>,
    pub data: U<8>,
}

/// A `let addr` bound before a wait, then, after it, a struct literal
/// whose field is named `addr` and does not read the local. The field
/// name was taken for a use of the local across the wait, and the unit
/// was refused (issue 1024).
#[derive(Trace, Default)]
pub struct FieldAfterWait {
    pub n: Reg<U<8>>,
}

#[lower]
impl Unit for FieldAfterWait {
    async fn run(&mut self, _inp: (), out: Tx<Word>) {
        loop {
            DefaultClock::rising().await;
            let addr = self.n.get() + 1;
            self.n.set(addr);
            DefaultClock::rising().await;
            out.send(Word {
                addr: U::<8>::from(7u8),
                data: self.n.get(),
            });
        }
    }
}

#[test]
fn a_field_named_as_an_earlier_let_is_not_a_use_of_it() {
    let net = FieldAfterWait::lowered("field_after_wait");
    assert!(net.verilog().contains("module field_after_wait"));
}
