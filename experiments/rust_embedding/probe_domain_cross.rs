// Probe 20b. A signal in one clock handed to a unit in another, with no
// crossing. Expected to fail. Both cases: a named clock into a
// different named clock, and the default clock into a named one, which
// shows the default is a domain and not a wildcard.
use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;

pub trait Wire: Copy + Default {}
impl Wire for u32 {}

pub trait Clock {}
pub struct DefaultClock; impl Clock for DefaultClock {}
pub struct Clk100; impl Clock for Clk100 {}
pub struct Clk400; impl Clock for Clk400 {}

struct Cellf<T: Wire>(Cell<T>);
pub struct Out<T: Wire, C: Clock = DefaultClock>(Rc<Cellf<T>>, PhantomData<C>);
pub struct In<T: Wire, C: Clock = DefaultClock>(Rc<Cellf<T>>, PhantomData<C>);

pub fn signal<T: Wire, C: Clock>() -> (Out<T, C>, In<T, C>) {
    let w = Rc::new(Cellf(Cell::new(T::default())));
    (Out(w.clone(), PhantomData), In(w, PhantomData))
}

pub struct Dsp { pub inp: In<u32, Clk400> }

pub fn wrong() -> Dsp {
    let (_tx, rx) = signal::<u32, Clk100>();
    Dsp { inp: rx }          // Clk100 into a Clk400 port
}

pub fn also_wrong() -> Dsp {
    let (_tx, rx) = signal::<u32, DefaultClock>();
    Dsp { inp: rx }          // the default clock into a Clk400 port
}
