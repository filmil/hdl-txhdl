// SPDX-License-Identifier: Apache-2.0
//! The experiment the embedding article states: one `mac`, lowered
//! twice with two multiplier latencies. `#[pipeline]` keeps the async
//! fn as written for simulation and emits its Verilog beside it. Each
//! awaited operator is a stage of the latency the attribute gives it;
//! `c`, ready at the input and used after the multiplier, is delayed by
//! as many registers as the multiplier takes, which is what a stage
//! boundary used to store by hand. The two lowerings differ in exactly
//! that: the depth of the multiplier and the length of `c`'s delay.
//! The async fn is also run through `pipeline::drive`, so it has a
//! waveform; the runtime's `mul` takes one cycle, so the waveform is
//! the first lowering's.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, join2, now, Clock, DefaultClock, Reg, Rx, Tx, Unit};
use txhdl::pipeline::{add, drive, mul};
use txhdl::types::U;
use txhdl::{pipeline, Transaction, Value};

/// A multiply-accumulate with a one-cycle multiplier.
#[pipeline(mul = 1, add = 1)]
async fn mac(a: U<32>, b: U<32>, c: U<64>) -> U<64> {
    let p = mul(a, b).await;
    add(p, c).await
}

/// The same shape with both constants written as literals rather than
/// taken as inputs: the factor in hexadecimal with the suffix that
/// says its width, and the addend in binary. A literal argument is
/// read the way Rust reads it, in any radix and with or without a
/// suffix, which is issue 200.
#[pipeline(mul = 1, add = 1)]
async fn scale(a: U<32>) -> U<64> {
    let p = mul(a, U::<32>::from(0x10u32)).await;
    add(p, U::<64>::from(0b1000_0000u64)).await
}

/// The same, with a three-cycle multiplier.
#[pipeline(mul = 3, add = 1)]
async fn mac3(a: U<32>, b: U<32>, c: U<64>) -> U<64> {
    let p = mul(a, b).await;
    add(p, c).await
}

#[derive(Transaction, Value, Clone, Copy, Default)]
pub struct Triple {
    pub a: U<8>,
    pub b: U<8>,
    pub c: U<8>,
}

/// Offers `(n, n, 100)` every cycle.
#[derive(Default)]
pub struct Source {
    pub n: Reg<U<8>>,
}

impl Unit<(), Tx<Triple>> for Source {
    async fn run(&mut self, _i: (), out: Tx<Triple>) {
        loop {
            DefaultClock::rising().await;
            let n = self.n.get();
            if out.ready().to_bool() {
                out.send(Triple {
                    a: n,
                    b: n,
                    c: U::from(100u8),
                });
                self.n.set(n + 1);
            }
        }
    }
}

#[derive(Default)]
pub struct Sink;

impl Unit<Rx<U<64>>, ()> for Sink {
    async fn run(&mut self, inp: Rx<U<64>>, _o: ()) {
        loop {
            let v = inp.wait().await;
            println!("t={:>2} out {}", now(), v.raw());
        }
    }
}

fn main() {
    println!("{}", mac_verilog());
    println!("{}", mac3_verilog());
    println!("{}", scale_verilog());

    let (t_tx, t_rx) = chan::<Triple, _>();
    let (r_tx, r_rx) = chan::<U<64>, _>();
    let mut source = Source::default();
    let mut sink = Sink;
    if let Some(mut vcd) = Wave::from_env() {
        vcd.clock::<DefaultClock>();
        vcd.add("triple", &t_rx);
        vcd.add("result", &r_rx);
        vcd.start();
    }
    let wide = |x: U<8>| U::<32>::from(x.raw() as u32);
    let f = move |t: Triple| {
        mac(wide(t.a), wide(t.b), U::<64>::from(t.c.raw() as u64))
    };
    let mut sim = txhdl::comp::Running::new(join2(
        join2(source.run((), t_tx), drive(f, t_rx, r_tx)),
        sink.run(r_rx, ()),
    ));
    for _ in 0..6 {
        sim.cycle();
    }
    let _ = (mac3, scale);
    stop();
}
