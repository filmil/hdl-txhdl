// SPDX-License-Identifier: Apache-2.0
// Probe 19b. A role that forgets a member. Expected to fail with the
// macro's own message naming the role and the member.
use txhdl::comp::{Chan, Signal};
use txhdl::interface;
use txhdl::types::{Bit, U};
use txhdl::Transaction;

#[derive(Clone, Copy, Default, Transaction)]
pub struct Beat {
    pub data: U<32>,
}

interface! {
    Wishbone { adr: Signal<U<32>>, ack: Signal<Bit>, dat: Chan<Beat> }
    role Initiator { out adr, in ack, out dat }
    role Target    { in adr,  out ack }          // dat is missing
}
