// Probe 19b. A role that forgets a member. The macro_rules! version
// compiled this silently. The proc macro must refuse it and say which
// role and which member.
use txhdl_interface::interface;
use wire::{Beat, Chan, Member, Signal};

interface! {
    Wishbone {
        adr: Signal<u32>,
        ack: Signal<bool>,
        dat: Chan<Beat>,
    }
    role Initiator { out adr, in ack, out dat }
    role Target    { in adr,  out ack }       // dat is missing
}
