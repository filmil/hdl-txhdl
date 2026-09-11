// Probe 19c. Two roles both driving one member. Expected to fail with the
// macro's message, which names both roles; without the check it would
// still fail, as E0382 on the second move of the driver.
use txhdl_interface::interface;
use wire::{Chan, Member, Signal};

interface! {
    Bad {
        adr: Signal<u32>,
        dat: Chan<u32>,
    }
    role A { out adr, out dat }
    role B { out adr, in dat }     // adr driven twice
}
