// SPDX-License-Identifier: Apache-2.0
// Probe 17. Nested configuration: a parent's config names its children's
// as associated types, so two children may share a knob name and a
// child's config is reusable. Against the library's `Config`.
use txhdl::comp::{rising, Config, DefaultClock, Reg, Unit};
use txhdl::types::U;

pub trait Generator: Default {
    fn next(&self) -> U<32>;
}
#[derive(Default)]
pub struct Counter;
impl Generator for Counter {
    fn next(&self) -> U<32> {
        U::new(1)
    }
}
#[derive(Default)]
pub struct Lfsr;
impl Generator for Lfsr {
    fn next(&self) -> U<32> {
        U::new(0x9E37)
    }
}

pub trait ProducerConfig {
    type Gen: Generator;
    const DEPTH: usize;
}
pub trait ConsumerConfig {
    const DEPTH: usize;
    const CHECKSUM: bool;
}
pub trait TopConfig: Config {
    type P: ProducerConfig;
    type C: ConsumerConfig;
}

/// One alias per level of the tree, written once beside the trait.
pub type ProducerOf<T> = <T as TopConfig>::P;
pub type ConsumerOf<T> = <T as TopConfig>::C;

#[derive(Default)]
pub struct Producer<PC: ProducerConfig> {
    pub gen: PC::Gen,
}
#[derive(Default)]
pub struct Consumer<CC: ConsumerConfig> {
    pub seen: Reg<U<32>>,
    _c: core::marker::PhantomData<CC>,
}
#[derive(Default)]
pub struct Top<TC: TopConfig> {
    pub producer: Producer<TC::P>,
    pub consumer: Consumer<TC::C>,
}

impl<TC: TopConfig> Unit<(), ()> for Top<TC> {
    async fn run(&mut self, _i: (), _o: ()) {
        let _ = (
            <ProducerOf<TC> as ProducerConfig>::DEPTH,
            <ConsumerOf<TC> as ConsumerConfig>::DEPTH,
        );
        loop {
            rising::<DefaultClock>().await;
            let seen = self.consumer.seen.get();
            self.consumer.seen.set(seen + self.producer.gen.next());
        }
    }
}

#[derive(Default)]
pub struct FastProducer;
impl ProducerConfig for FastProducer {
    type Gen = Lfsr;
    const DEPTH: usize = 16;
}
#[derive(Default)]
pub struct SmallProducer;
impl ProducerConfig for SmallProducer {
    type Gen = Counter;
    const DEPTH: usize = 2;
}
/// Written once, used by both builds. Same knob name as the producer's.
#[derive(Default)]
pub struct CheckedConsumer;
impl ConsumerConfig for CheckedConsumer {
    const DEPTH: usize = 64;
    const CHECKSUM: bool = true;
}

#[derive(Default)]
pub struct Fpga;
impl TopConfig for Fpga {
    type P = FastProducer;
    type C = CheckedConsumer;
}
impl Config for Fpga {
    type Top = Top<Fpga>;
    const NAME: &'static str = "fpga";
}
#[derive(Default)]
pub struct Tiny;
impl TopConfig for Tiny {
    type P = SmallProducer;
    type C = CheckedConsumer;
}
impl Config for Tiny {
    type Top = Top<Tiny>;
    const NAME: &'static str = "tiny";
}

/// The long spelling, and the aliased one. Both compile-time: an array
/// length accepts nothing else.
pub const LONG: usize = <<Fpga as TopConfig>::P as ProducerConfig>::DEPTH;
pub const SHORT: usize = <ProducerOf<Fpga> as ProducerConfig>::DEPTH;
pub type FpgaProducer = ProducerOf<Fpga>;
pub const SHORTER: usize = <FpgaProducer as ProducerConfig>::DEPTH;
pub static BUF: [u32; SHORTER] = [0; SHORTER];
