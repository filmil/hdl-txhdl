// Probe 14b. Driving an input. Expected to fail: direction is enforced by
// which methods a view exposes, so there is no `set` to call. Self
// contained, because an `include!` of a sibling source does not resolve
// inside the build sandbox.
use core::cell::Cell;

pub struct Signal<T: Copy + Default> { v: Cell<T> }

impl<T: Copy + Default> Signal<T> {
    pub fn new() -> Self { Signal { v: Cell::new(T::default()) } }
    pub(crate) fn read(&self) -> T { self.v.get() }
    pub(crate) fn drive(&self, x: T) { self.v.set(x) }
}

pub struct In<'a, T: Copy + Default>(pub(crate) &'a Signal<T>);
pub struct Out<'a, T: Copy + Default>(pub(crate) &'a Signal<T>);

impl<'a, T: Copy + Default> In<'a, T> { pub fn get(&self) -> T { self.0.read() } }
impl<'a, T: Copy + Default> Out<'a, T> { pub fn set(&self, x: T) { self.0.drive(x) } }

pub struct TargetView<'a> { pub adr: In<'a, u32> }

pub fn illegal(p: &TargetView<'_>) {
    p.adr.set(1);   // `adr` is In for a Target. There is no `set`.
}
