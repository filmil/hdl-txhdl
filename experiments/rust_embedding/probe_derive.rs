// SPDX-License-Identifier: Apache-2.0
// Probe 8. The derives, from the runtime's own macro crate. The bounds
// hold only if the derives emitted the impls.
use txhdl::comp::Bus as BusTrait;
use txhdl::types::Transaction as TransactionTrait;
use txhdl::{Bus, Transaction};

#[derive(Clone, Copy, Default, Transaction)]
pub struct MacRequest { pub addr: u32, pub count: u8 }

#[derive(Clone, Copy, Default, Transaction)]
pub struct MacResponse { pub acc: u64 }

#[derive(Bus)]
pub struct MemBus { pub req: MacRequest }

fn needs_transaction<T: TransactionTrait>(_t: &T) {}
fn needs_bus<B: BusTrait>(_b: &B) {}

pub fn check() {
    needs_transaction(&MacRequest { addr: 0, count: 1 });
    needs_transaction(&MacResponse { acc: 0 });
    needs_bus(&MemBus { req: MacRequest { addr: 1, count: 2 } });
}
