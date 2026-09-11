// SPDX-License-Identifier: Apache-2.0
//! An interface is declared once. The roles are generated, and a role
//! that omits a member, or two roles that both drive one, are refused
//! by the macro with a message naming them.
use txhdl::comp::{settle, Chan, Signal};
use txhdl::interface;
use txhdl::types::{Bit, U};
use txhdl::Transaction;

#[derive(Clone, Copy, Default, Transaction)]
pub struct Beat {
    pub data: U<32>,
    pub last: bool,
}

interface! {
    Wishbone {
        adr: Signal<U<32>>,
        ack: Signal<Bit>,
        dat: Chan<Beat>,
    }
    role Initiator { out adr, in ack,  out dat }
    role Target    { in adr,  out ack, in dat }
    role Monitor   { in adr,  in ack,  in dat }
}

pub fn transfer() -> (U<32>, Bit, Option<Beat>, U<32>) {
    let (m, t, mon) = Wishbone::new();
    m.adr.set(0x1000);
    m.dat.send(Beat {
        data: 7.into(),
        last: true,
    });
    t.ack.set(Bit::One);
    settle(); // the beat is in the channel at the next edge
    (t.adr.get(), m.ack.get(), t.dat.recv(), mon.adr.get())
}
