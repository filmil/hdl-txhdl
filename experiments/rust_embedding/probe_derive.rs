// Probe 8. Does a derive give the "kind before data" reading that putting
// the impl first was reaching for, while staying idiomatic Rust?

pub mod types {
    pub trait Transaction {}
}

pub mod comp {
    pub trait Bus {}
}

use txhdl_derive::{Bus, Transaction};

#[derive(Transaction)]
pub struct MacRequest {
    pub addr: u32,
    pub count: u8,
}

#[derive(Transaction)]
pub struct MacResponse {
    pub acc: u64,
}

#[derive(Bus)]
pub struct MemBus {
    pub req: MacRequest,
}

/// Proof the impls exist: these bounds only hold if the derives ran.
fn needs_transaction<T: types::Transaction>(_t: &T) {}
fn needs_bus<B: comp::Bus>(_b: &B) {}

pub fn check() {
    let r = MacRequest { addr: 0, count: 1 };
    let s = MacResponse { acc: 0 };
    let b = MemBus { req: MacRequest { addr: 1, count: 2 } };
    needs_transaction(&r);
    needs_transaction(&s);
    needs_bus(&b);
}
