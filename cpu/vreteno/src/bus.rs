// SPDX-License-Identifier: Apache-2.0
//! The bus: one channel each way between the core and its devices. A
//! request is one word of 69 bits, the address, the data to write,
//! the four lane enables and whether it is a write, packed high to low;
//! a response is the word read. A request is a transaction, so it
//! moves as a channel moves, one per cycle, registered on both sides,
//! and a device answers a read in the cycle after it takes the
//! request. A struct would say the same and be the same bits; the
//! word is what the lowering slices.
use txhdl::types::{Bit, U};

/// The request's width and the offsets of its fields.
pub const REQ_BITS: usize = 69;
pub const REQ_ADDR: usize = 37;
pub const REQ_WDATA: usize = 5;
pub const REQ_LANES: usize = 1;
pub const REQ_WE: usize = 0;

/// Pack a request, the Rust way, for a testbench or a model.
pub fn pack(addr: u32, wdata: u32, lanes: u8, we: bool) -> U<REQ_BITS> {
    let a = addr as u128;
    let w = wdata as u128;
    let l = (lanes & 15) as u128;
    U::new((a << REQ_ADDR) | (w << REQ_WDATA) | (l << REQ_LANES) | we as u128)
}

/// A request's fields, the Rust way.
pub fn unpack(r: U<REQ_BITS>) -> (u32, u32, u8, bool) {
    let v = r.raw();
    (
        (v >> REQ_ADDR) as u32,
        (v >> REQ_WDATA) as u32,
        (v >> REQ_LANES) as u8 & 15,
        v & 1 == 1,
    )
}

/// The lane enables as a bit each.
pub fn lane(r: U<REQ_BITS>, i: usize) -> Bit {
    r.bit(REQ_LANES + i)
}
