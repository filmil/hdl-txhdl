// Probe 19c. Two roles driving one member. Expected to fail with the
// macro's message naming both roles.
use txhdl::comp::{Chan, Signal};
use txhdl::interface;
use txhdl::types::U;
use txhdl::Transaction;

#[derive(Clone, Copy, Default, Transaction)]
pub struct Beat { pub data: U<32> }

interface! {
    Bad { adr: Signal<U<32>>, dat: Chan<Beat> }
    role A { out adr, out dat }
    role B { out adr, in dat }
}
