// Probe 16b. Two drivers on one wire. Expected to fail: `Out<T>` is not
// `Clone` and `signal()` returns exactly one, so a second driver cannot
// be produced. Single-driver is a move, not a rule.
use std::cell::Cell;
use std::rc::Rc;

struct Wire<T: Copy> { v: Cell<T> }
pub struct Out<T: Copy> { w: Rc<Wire<T>> }
pub struct In<T: Copy> { w: Rc<Wire<T>> }

impl<T: Copy> Out<T> { pub fn set(&self, v: T) { self.w.v.set(v) } }
impl<T: Copy> In<T> { pub fn get(&self) -> T { self.w.v.get() } }

pub fn signal<T: Copy + Default>() -> (Out<T>, In<T>) {
    let w = Rc::new(Wire { v: Cell::new(T::default()) });
    (Out { w: w.clone() }, In { w })
}

pub struct Driver { pub out: Out<u32> }

pub fn two_drivers() -> (Driver, Driver) {
    let (tx, _rx) = signal::<u32>();
    // The first move is fine. The second has nothing left to move.
    let a = Driver { out: tx };
    let b = Driver { out: tx };
    (a, b)
}
