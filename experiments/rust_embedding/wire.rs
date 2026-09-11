// The wire types the interface macro emits code against. A library so
// that several probes share one definition and the macro's output has a
// `Member` to name.
use std::cell::Cell;
use std::rc::Rc;

pub trait Wire: Copy + Default {}
impl Wire for u32 {}
impl Wire for bool {}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Beat { pub data: u32, pub last: bool }
impl Wire for Beat {}

pub struct Cellf<T: Wire>(pub Cell<T>);

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

/// What every member of an interface can do. The macro calls only this.
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
