// SPDX-License-Identifier: Apache-2.0
// Probe 19. The library's `interface!`, a procedural macro: any number
// of roles, `in` as written, and a completeness check.
use txhdl::comp::{Chan, Signal};
use txhdl::interface;
use txhdl::types::{Bit, U};
use txhdl::Transaction;

#[derive(Clone, Copy, Default, Transaction)]
pub struct Beat { pub data: U<32>, pub last: bool }

interface! {
    Wishbone { adr: Signal<U<32>>, ack: Signal<Bit>, dat: Chan<Beat> }
    role Initiator { out adr, in ack,  out dat }
    role Target    { in adr,  out ack, in dat }
    role Monitor   { in adr,  in ack,  in dat }
}

pub fn check() -> (U<32>, Bit, Option<Beat>, U<32>) {
    let (m, t, mon) = Wishbone::new();
    m.adr.set(U::new(0x1000));
    m.dat.send(Beat { data: U::new(7), last: true });
    t.ack.set(Bit::One);
    (t.adr.get(), m.ack.get(), t.dat.recv(), mon.adr.get())
}
