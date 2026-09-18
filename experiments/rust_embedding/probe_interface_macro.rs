// SPDX-License-Identifier: Apache-2.0
// Probe 18. The `macro_rules!` interface macro, kept as the record of
// where that route stopped: exactly two roles, `inp` for `in`, and a
// role that omits a member compiles silently. Against the library's
// `Member`, `Signal` and `Chan`. The library's `interface!` is a
// procedural macro for these three reasons; see probe 19.
use txhdl::comp::{Chan, Member, Signal};
use txhdl::types::{Bit, Transaction, U};

#[derive(Clone, Copy, Default)]
pub struct Beat {
    pub data: U<32>,
    pub last: bool,
}
impl Transaction for Beat {}

macro_rules! end_ty {
    (out, $t:ty) => {
        <$t as Member>::Driver
    };
    (inp, $t:ty) => {
        <$t as Member>::Reader
    };
}
macro_rules! end_of {
    (out, $pair:expr) => {
        $pair.0
    };
    (inp, $pair:expr) => {
        $pair.1.clone()
    };
}
macro_rules! interface_rules {
    ($iface:ident { $( $m:ident : $ty:ty ),* $(,)? }
     role $ra:ident { $( $da:ident $fa:ident ),* $(,)? }
     role $rb:ident { $( $db:ident $fb:ident ),* $(,)? }) => {
        pub struct $iface;
        pub struct $ra { $( pub $fa: end_ty!($da, $ty), )* }
        pub struct $rb { $( pub $fb: end_ty!($db, $ty), )* }
        impl $iface {
            #[allow(clippy::new_ret_no_self)]
            pub fn new() -> ($ra, $rb) {
                $( let $m = <$ty as Member>::new().split(); )*
                ( $ra { $( $fa: end_of!($da, $fa), )* }, $rb { $( $fb: end_of!($db, $fb), )* } )
            }
        }
    };
}

interface_rules! {
    Wishbone { adr: Signal<U<32>>, ack: Signal<Bit>, dat: Chan<Beat> }
    role Initiator { out adr, inp ack, out dat }
    role Target    { inp adr, out ack, inp dat }
}

pub fn check() -> (U<32>, Bit, Option<Beat>) {
    let (m, t) = Wishbone::new();
    m.adr.set(U::new(0x1000));
    m.dat.send(Beat {
        data: U::new(7),
        last: true,
    });
    t.ack.set(Bit::One);
    (t.adr.get(), m.ack.get(), t.dat.recv())
}
