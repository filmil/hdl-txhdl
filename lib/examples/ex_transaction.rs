// SPDX-License-Identifier: Apache-2.0
//! A transaction is a struct and a derive. A channel carries one.
use txhdl::comp::{chan, settle, DefaultClock, Rx, Tx};
use txhdl::types::U;
use txhdl::Transaction;

#[derive(Clone, Copy, Default, Transaction)]
pub struct MacRequest {
    pub addr: U<32>,
    pub count: U<8>,
}

#[derive(Clone, Copy, Default, Transaction)]
pub struct MacResponse {
    pub acc: U<64>,
}

/// Creating a channel yields its two ends. The sender is unique; the
/// receiver clones.
pub fn wire_up() -> (Tx<MacRequest>, Rx<MacRequest>, Rx<MacRequest>) {
    let (tx, rx) = chan::<MacRequest, DefaultClock>();
    let tap = rx.clone();
    (tx, rx, tap)
}

pub fn exchange() -> Option<MacRequest> {
    let (tx, rx, tap) = wire_up();
    tx.send(MacRequest {
        addr: 0x1000.into(),
        count: 4.into(),
    });
    settle(); // the offer is in the channel at the next edge
    let _ = tap.peek(); // fanout: the tap sees the same head
    rx.recv()
}
