// SPDX-License-Identifier: Apache-2.0
// Probe 7. May an `impl` precede the `struct` it is for? Items in a Rust
// module are not order dependent, unlike statements in a block.
use txhdl::comp::Bus;
use txhdl::types::Transaction;

impl Transaction for MacRequest {}

#[derive(Clone, Copy, Default)]
pub struct MacRequest { pub addr: u32, pub count: u8 }

impl Named for MacResponse { fn name(&self) -> &'static str { "MacResponse" } }
pub trait Named { fn name(&self) -> &'static str; }
#[derive(Clone, Copy, Default)]
pub struct MacResponse { pub acc: u64 }

impl Bus for MemBus {}
pub struct MemBus { pub req: MacRequest }

impl<T: Transaction> Holder<T> { pub fn new(v: T) -> Self { Holder { v } } }
pub struct Holder<T: Transaction> { pub v: T }

pub fn build() -> Holder<MacRequest> { Holder::new(MacRequest { addr: 0, count: 1 }) }
