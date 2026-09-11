// Probe 14. Can direction be enforced rather than documented? Against the
// library's ends: `In` has `get` and no `set`; `Out` has `set` and no
// `get`. A unit given the wrong end is a type error at the call.
use txhdl::comp::{signal, DefaultClock, In, Out};
use txhdl::types::U;

pub struct Cpu { pub adr: Out<U<32>>, pub ack: In<U<1>> }
pub struct Ram { pub adr: In<U<32>>, pub ack: Out<U<1>> }

pub fn build() -> (Cpu, Ram) {
    let (adr_out, adr_in) = signal::<U<32>, DefaultClock>();
    let (ack_out, ack_in) = signal::<U<1>, DefaultClock>();
    (Cpu { adr: adr_out, ack: ack_in }, Ram { adr: adr_in, ack: ack_out })
}

pub fn step(cpu: &Cpu, ram: &Ram) -> (U<32>, U<1>) {
    cpu.adr.set(U::new(0x1000));
    ram.ack.set(U::new(1));
    (ram.adr.get(), cpu.ack.get())
}
