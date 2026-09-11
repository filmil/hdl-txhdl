// Probe 17. Nested configuration.
//
// Probe 6 gave one config for a whole design, with a nested unit generic
// over the same config. That works while every choice is global. It stops
// working as soon as two children want a knob of the same name, and it
// makes a child's configuration unreusable outside its parent.
//
// The fix is that a config is a tree: a parent's config names its
// children's configs as associated types. A child then has a
// configuration of its own, which can be written once and used by several
// parents.

pub trait Generator { fn new() -> Self; fn next(&self) -> u32; }

pub struct Counter;
impl Generator for Counter {
    fn new() -> Self { Counter }
    fn next(&self) -> u32 { 1 }
}

pub struct Lfsr;
impl Generator for Lfsr {
    fn new() -> Self { Lfsr }
    fn next(&self) -> u32 { 0x9E37 }
}

// --- each unit declares the configuration it needs --------------------

pub trait ProducerConfig {
    type Gen: Generator;
    /// Note the name.
    const DEPTH: usize;
}

pub trait ConsumerConfig {
    /// The same name, a different unit, and no collision, because the two
    /// live in different traits.
    const DEPTH: usize;
    const CHECKSUM: bool;
}

/// The parent's configuration names its children's, rather than holding
/// their fields.
pub trait TopConfig {
    type P: ProducerConfig;
    type C: ConsumerConfig;
    const NAME: &'static str;
}

// --- units are generic over their own config only ---------------------

pub struct Producer<PC: ProducerConfig> {
    pub gen: PC::Gen,
    pub buf: [u32; 4],
}

impl<PC: ProducerConfig> Producer<PC> {
    pub fn new() -> Self { Producer { gen: PC::Gen::new(), buf: [0; 4] } }
    pub fn depth(&self) -> usize { PC::DEPTH }
    pub fn step(&self) -> u32 { self.gen.next() }
}

pub struct Consumer<CC: ConsumerConfig> {
    pub seen: u32,
    pub _c: core::marker::PhantomData<CC>,
}

impl<CC: ConsumerConfig> Consumer<CC> {
    pub fn new() -> Self { Consumer { seen: 0, _c: core::marker::PhantomData } }
    pub fn depth(&self) -> usize { CC::DEPTH }
    pub fn checks(&self) -> bool { CC::CHECKSUM }
}

/// The parent reaches each child's config through its own, and never
/// mentions a child's knobs.
pub struct Top<TC: TopConfig> {
    pub producer: Producer<TC::P>,
    pub consumer: Consumer<TC::C>,
}

impl<TC: TopConfig> Top<TC> {
    pub fn new() -> Self {
        Top { producer: Producer::<TC::P>::new(), consumer: Consumer::<TC::C>::new() }
    }
    pub fn name(&self) -> &'static str { TC::NAME }
}

// --- configurations, written once and reused --------------------------

pub struct FastProducer;
impl ProducerConfig for FastProducer {
    type Gen = Lfsr;
    const DEPTH: usize = 16;
}

pub struct SmallProducer;
impl ProducerConfig for SmallProducer {
    type Gen = Counter;
    const DEPTH: usize = 2;
}

/// One child configuration, used by both builds below. That reuse is what
/// the flat version could not express.
pub struct CheckedConsumer;
impl ConsumerConfig for CheckedConsumer {
    const DEPTH: usize = 64;      // same name as ProducerConfig::DEPTH
    const CHECKSUM: bool = true;
}

pub struct Fpga;
impl TopConfig for Fpga {
    type P = FastProducer;
    type C = CheckedConsumer;
    const NAME: &'static str = "fpga";
}

pub struct Tiny;
impl TopConfig for Tiny {
    type P = SmallProducer;
    type C = CheckedConsumer;   // reused, unchanged
    const NAME: &'static str = "tiny";
}

// --- the two builds ---------------------------------------------------

pub type FpgaBuild = Top<Fpga>;
pub type TinyBuild = Top<Tiny>;

/// The two DEPTHs coexist and differ, which is the point.
pub fn depths<TC: TopConfig>() -> (usize, usize) {
    let t = Top::<TC>::new();
    (t.producer.depth(), t.consumer.depth())
}

pub fn check() -> ((usize, usize), (usize, usize)) {
    (depths::<Fpga>(), depths::<Tiny>())   // ((16, 64), (2, 64))
}

/// Compile-time, so a child's constant may still size an array in the
/// parent.
pub const FPGA_PRODUCER_DEPTH: usize = <<Fpga as TopConfig>::P as ProducerConfig>::DEPTH;
pub static FPGA_BUF: [u32; FPGA_PRODUCER_DEPTH] = [0; FPGA_PRODUCER_DEPTH];
