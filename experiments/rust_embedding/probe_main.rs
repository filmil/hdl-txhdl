// Probe 13. A design is a program: `fn main()` instantiates the top unit
// through a configuration.
//
// This closes the hierarchy. A unit contains units (probe 12); the
// outermost one is named by a config, and `main` asks for it by name.
// There is no separate elaboration entry point and no build description
// language: the design is a Rust program that you run.

use core::cell::Cell;
use core::future::Future;
use core::task::{Context, RawWaker, RawWakerVTable, Waker};

pub mod comp {
    #[allow(async_fn_in_trait)]
    pub trait Module<In, Out> {
        async fn run(&mut self, inputs: In, outputs: Out);
    }

    /// A named build. It says which design to elaborate as well as what
    /// to fill it with, so a config is the whole of what `main` needs.
    pub trait Config {
        type Top: Module<(), ()>;
        const NAME: &'static str;
        const CLK_HZ: u64;
        fn top() -> Self::Top;
    }
}

use comp::{Config, Module};

// --- the smallest executor that can drive a design -------------------

fn noop_waker() -> Waker {
    fn nop(_: *const ()) {}
    fn clone(_: *const ()) -> RawWaker { RawWaker::new(core::ptr::null(), &VT) }
    static VT: RawWakerVTable = RawWakerVTable::new(clone, nop, nop, nop);
    unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VT)) }
}

/// One cycle is one poll of the top unit's future.
pub fn step<F: Future<Output = ()>>(f: F) {
    let mut f = Box::pin(f);
    let w = noop_waker();
    let mut cx = Context::from_waker(&w);
    let _ = f.as_mut().poll(&mut cx);
}

// --- a design --------------------------------------------------------

pub trait MacOp { fn new() -> Self; fn mul(&self, a: u32, b: u32) -> u64; }

pub struct DspSliceMac;
impl MacOp for DspSliceMac {
    fn new() -> Self { DspSliceMac }
    fn mul(&self, a: u32, b: u32) -> u64 { a as u64 * b as u64 }
}

pub struct WallaceTreeMac;
impl MacOp for WallaceTreeMac {
    fn new() -> Self { WallaceTreeMac }
    fn mul(&self, a: u32, b: u32) -> u64 { a as u64 * b as u64 }
}

pub struct Top<C: Config2> {
    pub mac: C::Mac,
    pub acc: Cell<u64>,
}

/// Split out so that `Config` can name `Top<C>` without the two traits
/// referring to each other in a circle.
pub trait Config2 {
    type Mac: MacOp;
    const TAPS: usize;
}

impl<C: Config2> Top<C> {
    pub fn new() -> Self { Top { mac: C::Mac::new(), acc: Cell::new(0) } }
}

impl<C: Config2> Module<(), ()> for Top<C> {
    async fn run(&mut self, _i: (), _o: ()) {
        for i in 0..C::TAPS {
            self.acc.set(self.acc.get() + self.mac.mul(i as u32, 2));
        }
    }
}

// --- two builds ------------------------------------------------------

pub struct Fpga;
impl Config2 for Fpga { type Mac = DspSliceMac; const TAPS: usize = 8; }
impl Config for Fpga {
    type Top = Top<Fpga>;
    const NAME: &'static str = "fpga";
    const CLK_HZ: u64 = 100_000_000;
    fn top() -> Self::Top { Top::<Fpga>::new() }
}

pub struct Asic;
impl Config2 for Asic { type Mac = WallaceTreeMac; const TAPS: usize = 64; }
impl Config for Asic {
    type Top = Top<Asic>;
    const NAME: &'static str = "asic";
    const CLK_HZ: u64 = 400_000_000;
    fn top() -> Self::Top { Top::<Asic>::new() }
}

// --- what a real toolchain would export ------------------------------

/// Elaborate the design a config names. A real one emits a netlist; this
/// one runs a cycle and reports, which is enough to show that `main`
/// reaches the design through nothing but the config.
pub fn elaborate<C: Config>() {
    let mut top = C::top();
    step(top.run((), ()));
    println!(
        "elaborated config '{}' at {} MHz",
        C::NAME,
        C::CLK_HZ / 1_000_000
    );
}

// --- the entry point -------------------------------------------------

fn main() {
    // A design is a program. Which build to elaborate is an argument,
    // and each arm is monomorphised separately.
    let which = std::env::args().nth(1).unwrap_or_else(|| "fpga".to_string());
    match which.as_str() {
        "asic" => elaborate::<Asic>(),
        _ => elaborate::<Fpga>(),
    }
}
