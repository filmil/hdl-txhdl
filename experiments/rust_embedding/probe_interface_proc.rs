// Probe 19. The interface! proc macro. Three roles, `in` as the keyword,
// and the same declaration otherwise as the macro_rules! version.
use txhdl_interface::interface;
use wire::{Beat, Chan, Member, Signal};

interface! {
    Wishbone {
        adr: Signal<u32>,
        ack: Signal<bool>,
        dat: Chan<Beat>,
    }
    role Initiator { out adr, in ack, out dat }
    role Target    { in adr,  out ack, in dat }
    role Monitor   { in adr,  in ack,  in dat }
}

pub fn check() -> (u32, bool, Beat, u32) {
    let (m, t, mon) = Wishbone::new();
    m.adr.set(0x1000);
    m.dat.send(Beat { data: 7, last: true });
    t.ack.set(true);
    (t.adr.get(), m.ack.get(), t.dat.recv(), mon.adr.get())
}
