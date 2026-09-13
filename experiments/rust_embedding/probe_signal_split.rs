// SPDX-License-Identifier: Apache-2.0
// Probe 16. A signal yields its two ends separately; `In` clones for
// fanout and `Out` does not. Against the library's `signal`.
use txhdl::comp::{signal, DefaultClock, In, Out, Reg};
use txhdl::types::U;

#[derive(Clone, Copy, Default)]
pub struct Beat { pub data: U<32>, pub last: bool }

pub struct Producer { pub out: Out<Beat>, pub n: Reg<U<32>> }
pub struct Consumer { pub inp: In<Beat>, pub seen: Reg<U<32>> }
pub struct Monitor  { pub inp: In<Beat>, pub last: Reg<U<32>> }

pub fn build() -> (Producer, Consumer, Monitor) {
    let (tx, rx) = signal::<Beat, DefaultClock>();
    (
        Producer { out: tx, n: Reg::new(U::new(0)) },
        Consumer { inp: rx.clone(), seen: Reg::new(U::new(0)) },
        Monitor  { inp: rx,         last: Reg::new(U::new(0)) },
    )
}
