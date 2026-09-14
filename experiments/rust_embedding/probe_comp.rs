// SPDX-License-Identifier: Apache-2.0
// Probe 3. Do `Bus`, `Chan`, `Signal`, `Member` and `Tag` hold together,
// and can a unit be generic over a tag? Against the library.
use txhdl::comp::{
    chan, rising, signal, DefaultClock, In, Out, Reg, Rx, Tx, Unit,
};
use txhdl::types::{Tag, Transaction, U};

#[derive(Clone, Copy, Default)]
pub struct MemRead {
    pub addr: U<32>,
}
impl Transaction for MemRead {}

pub struct MemFetch;
impl Tag for MemFetch {
    const HANDSHAKE: bool = true;
    const CAPACITY: usize = 8;
}

/// A unit names its domain as a type parameter, and reads the policy as
/// a compile-time constant.
pub struct MacUnit<T: Tag> {
    pub req: Tx<MemRead>,
    pub ack: In<U<1>>,
    pub acc: Reg<U<64>>,
    _t: core::marker::PhantomData<T>,
}

impl<T: Tag> Unit<U<32>, ()> for MacUnit<T> {
    async fn run(&mut self, addr: U<32>, _o: ()) {
        let _elastic = T::HANDSHAKE;
        loop {
            rising::<DefaultClock>().await;
            self.req.send(MemRead { addr });
            if self.ack.get().raw() == 1 {
                // a wire: no wait
                self.acc.set(self.acc + 1) // a register, read in place
            }
        }
    }
}

pub fn build() -> (MacUnit<MemFetch>, Rx<MemRead>, Out<U<1>>) {
    let (tx, rx) = chan::<MemRead, DefaultClock>();
    let (ack_out, ack_in) = signal::<U<1>, DefaultClock>();
    (
        MacUnit {
            req: tx,
            ack: ack_in,
            acc: Reg::new(U::new(0)),
            _t: core::marker::PhantomData,
        },
        rx,
        ack_out,
    )
}

pub type Elastic = MacUnit<MemFetch>;
pub type Raw = MacUnit<txhdl::types::Raw>;
