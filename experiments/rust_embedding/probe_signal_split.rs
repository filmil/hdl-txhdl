// Probe 16. `Signal<T>` as a channel with two ends, in the mpsc sense.
//
// Probe 14 derived direction from a shared reference and a role view.
// This is better: creating a signal yields its two ends as separate
// values, so a unit that holds an end cannot use the other one, and
// wiring is handing an end to whoever drives or reads it.
//
// Hardware inverts mpsc's cardinality, and the types say so:
//   mpsc      Sender is Clone, Receiver is unique
//   hardware  the driver is unique, readers fan out
//
// So `In<T>` is Clone and `Out<T>` is not. One driver per wire is then
// enforced by move semantics rather than by a rule in a document.

use std::cell::Cell;
use std::rc::Rc;

/// What a signal carries. A struct, so a wire may be a bundle without
/// needing a separate concept for the bundled case.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Beat {
    pub data: u32,
    pub last: bool,
}

/// The wire itself. Neither end, and not nameable by a unit: a unit only
/// ever holds an end.
struct Wire<T: Copy> {
    v: Cell<T>,
}

/// The driving end. Deliberately not `Clone`: a second driver would be a
/// second value, and there is no way to make one.
pub struct Out<T: Copy> {
    w: Rc<Wire<T>>,
}

/// The reading end. `Clone`, because fanout is normal and costs nothing.
#[derive(Clone)]
pub struct In<T: Copy> {
    w: Rc<Wire<T>>,
}

impl<T: Copy> Out<T> {
    pub fn set(&self, v: T) { self.w.v.set(v) }
}

impl<T: Copy> In<T> {
    pub fn get(&self) -> T { self.w.v.get() }
}

/// Create a signal. The two ends come back separately, which is the
/// whole point.
pub fn signal<T: Copy + Default>() -> (Out<T>, In<T>) {
    let w = Rc::new(Wire { v: Cell::new(T::default()) });
    (Out { w: w.clone() }, In { w })
}

// --- units hold ends, not wires --------------------------------------

pub struct Producer { pub out: Out<Beat>, pub n: Cell<u32> }
pub struct Consumer { pub inp: In<Beat>, pub seen: Cell<u32> }
pub struct Monitor  { pub inp: In<Beat>, pub last: Cell<u32> }

impl Producer {
    pub fn step(&self) {
        let n = self.n.get();
        self.out.set(Beat { data: n, last: n == 7 });
        self.n.set(n + 1);
    }
}

impl Consumer {
    pub fn step(&self) { if self.inp.get().last { self.seen.set(self.seen.get() + 1) } }
}

impl Monitor {
    pub fn step(&self) { self.last.set(self.inp.get().data) }
}

/// The parent creates the signal and hands out the ends. Two readers,
/// one driver, and nobody names the wire.
pub struct Top {
    pub producer: Producer,
    pub consumer: Consumer,
    pub monitor: Monitor,
}

impl Top {
    pub fn new() -> Self {
        let (tx, rx) = signal::<Beat>();
        Top {
            producer: Producer { out: tx, n: Cell::new(0) },
            consumer: Consumer { inp: rx.clone(), seen: Cell::new(0) },
            monitor:  Monitor  { inp: rx,         last: Cell::new(0) },
        }
    }

    pub fn cycle(&self) {
        self.producer.step();
        self.consumer.step();
        self.monitor.step();
    }
}

pub fn check() -> (u32, u32) {
    let t = Top::new();
    for _ in 0..8 { t.cycle() }
    (t.consumer.seen.get(), t.monitor.last.get())
}
