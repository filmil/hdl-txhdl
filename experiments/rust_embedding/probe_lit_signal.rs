// SPDX-License-Identifier: Apache-2.0
//! Probe 26. Expected to fail: an expression the lowering does not
//! recognise, which names a wire. `U::<8>::new(n)` is not one of the
//! forms the lowering reads, and the fallback is `lit`, a constant
//! evaluated where `lowered` runs. The wire `n` means nothing there,
//! so the macro refuses the expression by name rather than leaving
//! `rustc` to report a missing variable at the attribute (issue 128).
use txhdl::comp::{Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::U;
use txhdl::{lower, Trace};

#[derive(Trace, Default)]
pub struct Widen {
    pub count: Reg<U<8>>,
}

#[lower]
impl Unit for Widen {
    async fn run(&mut self, (en,): (In<U<1>>,), (q,): (Out<U<8>>,)) {
        loop {
            DefaultClock::rising().await;
            let n = self.count.get() + en.get().zext::<8>();
            q.set(U::<8>::new(n));
        }
    }
}
