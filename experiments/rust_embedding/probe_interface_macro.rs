// Probe 18. Declaring an interface and generating its role views.
//
// Two problems at once. The role structs written by hand are mechanical,
// and they are also the place an interface and its roles drift apart.
// And the hand-written version used borrowed views, which the mpsc split
// of probe 16 supersedes: an end should be an owned value, so that one
// driver per wire stays a fact about moves.
//
// So: `Member::split` consumes a member and returns its two ends, the
// macro never learns which kind of member it is holding, and an interface
// is constructed once and yields one port per role.

use std::cell::Cell;
use std::rc::Rc;

pub trait Wire: Copy + Default {}
impl Wire for u32 {}
impl Wire for bool {}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Beat { pub data: u32, pub last: bool }
impl Wire for Beat {}

struct Cellf<T: Wire>(Cell<T>);

// --- members and their ends ------------------------------------------

pub struct Signal<T: Wire>(Rc<Cellf<T>>);
pub struct Chan<T: Wire>(Rc<Cellf<T>>);

pub struct Out<T: Wire>(Rc<Cellf<T>>);
pub struct In<T: Wire>(Rc<Cellf<T>>);
pub struct Tx<T: Wire>(Rc<Cellf<T>>);
pub struct Rx<T: Wire>(Rc<Cellf<T>>);

impl<T: Wire> Clone for In<T> { fn clone(&self) -> Self { In(self.0.clone()) } }
impl<T: Wire> Clone for Rx<T> { fn clone(&self) -> Self { Rx(self.0.clone()) } }

impl<T: Wire> Out<T> { pub fn set(&self, v: T) { self.0 .0.set(v) } }
impl<T: Wire> In<T> { pub fn get(&self) -> T { self.0 .0.get() } }
impl<T: Wire> Tx<T> { pub fn send(&self, v: T) { self.0 .0.set(v) } }
impl<T: Wire> Rx<T> { pub fn recv(&self) -> T { self.0 .0.get() } }

/// What every member of an interface can do. The macro calls only this,
/// so it never has to know whether a member is a wire or a channel.
pub trait Member {
    type Driver;
    type Reader;
    fn new() -> Self;
    fn split(self) -> (Self::Driver, Self::Reader);
}

impl<T: Wire> Member for Signal<T> {
    type Driver = Out<T>;
    type Reader = In<T>;
    fn new() -> Self { Signal(Rc::new(Cellf(Cell::new(T::default())))) }
    fn split(self) -> (Out<T>, In<T>) { (Out(self.0.clone()), In(self.0)) }
}

impl<T: Wire> Member for Chan<T> {
    type Driver = Tx<T>;
    type Reader = Rx<T>;
    fn new() -> Self { Chan(Rc::new(Cellf(Cell::new(T::default())))) }
    fn split(self) -> (Tx<T>, Rx<T>) { (Tx(self.0.clone()), Rx(self.0)) }
}

/// Pick an end by direction. `inp` rather than `in`, because `in` is a
/// Rust keyword and reads as one at the call site.
#[macro_export]
macro_rules! end_ty {
    (out, $t:ty) => { <$t as $crate::Member>::Driver };
    (inp, $t:ty) => { <$t as $crate::Member>::Reader };
}

#[macro_export]
macro_rules! end_of {
    (out, $pair:expr) => { $pair.0 };
    (inp, $pair:expr) => { $pair.1 };
}

/// Declare an interface and the two roles over it. Everything below is
/// generated: the interface struct, one port struct per role, and a
/// constructor that yields exactly one port for each.
#[macro_export]
macro_rules! interface {
    (
        $iface:ident { $( $m:ident : $ty:ty ),* $(,)? }
        role $ra:ident { $( $da:ident $fa:ident ),* $(,)? }
        role $rb:ident { $( $db:ident $fb:ident ),* $(,)? }
    ) => {
        pub struct $iface;

        pub struct $ra { $( pub $fa: $crate::end_ty!($da, $ty), )* }
        pub struct $rb { $( pub $fb: $crate::end_ty!($db, $ty), )* }

        impl $iface {
            /// One port per role, produced once. A second driver would
            /// need a second call, and a second call makes a second
            /// interface rather than another end of this one.
            pub fn new() -> ($ra, $rb) {
                $( let $m = <$ty as $crate::Member>::new().split(); )*
                (
                    $ra { $( $fa: $crate::end_of!($da, $fa), )* },
                    $rb { $( $fb: $crate::end_of!($db, $fb), )* },
                )
            }
        }
    };
}

// --- the whole of VII.C, declared -------------------------------------

interface! {
    Wishbone {
        adr: Signal<u32>,
        ack: Signal<bool>,
        dat: Chan<Beat>,
    }
    role Initiator { out adr, inp ack, out dat }
    role Target    { inp adr, out ack, inp dat }
}

// --- and it works -----------------------------------------------------

pub fn check() -> (u32, bool, Beat) {
    let (m, t) = Wishbone::new();

    m.adr.set(0x1000);           // Initiator drives adr
    m.dat.send(Beat { data: 7, last: true });
    t.ack.set(true);             // Target drives ack

    (t.adr.get(), m.ack.get(), t.dat.recv())
}
