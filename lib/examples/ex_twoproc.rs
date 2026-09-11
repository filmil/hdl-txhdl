// SPDX-License-Identifier: Apache-2.0
//! Two processes in one unit, one on each edge, lowered. The counter
//! advances on the rising edge while enabled; the sampler copies it on
//! the falling edge, half a cycle later, and the output shows the
//! copy. `run` is `join2` of two loops, each a process of its own with
//! its own wait; the lowering makes a clocked block of each, one on
//! the rising edge and one on the falling, and the netlist is checked
//! against the trace at both.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    join2, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

#[derive(Trace, Default)]
pub struct Sampler {
    pub n: Reg<U<4>>,
    pub half: Reg<U<4>>,
}

#[lower]
impl Unit<In<Bit>, Out<U<4>>> for Sampler {
    async fn run(&mut self, enable: In<Bit>, q: Out<U<4>>) {
        let this = &*self;
        join2(
            async move {
                loop {
                    DefaultClock::rising().await;
                    let (n, en) = (this.n.get(), enable.get());
                    when!(en => { this.n <= n.wrapping_add(1) });
                }
            },
            async move {
                loop {
                    DefaultClock::falling().await;
                    let n = this.n.get();
                    this.half.set(n);
                    q.set(this.half.get());
                }
            },
        )
        .await;
    }
}

fn main() {
    let (enable_out, enable) = signal::<Bit, DefaultClock>();
    let (q_out, q) = signal::<U<4>, DefaultClock>();
    let mut unit = Sampler::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("enable", &enable);
        w.add("sampler", &unit);
        w.add("q", &q);
        w.start();
    }
    let mut sim = Running::new(unit.run(enable, q_out));
    enable_out.set(Bit::One);
    println!("  t  q");
    for cycle in 0..10 {
        if cycle == 6 {
            enable_out.set(Bit::Zero);
        }
        sim.cycle();
        println!("{:>3}  {}", txhdl::comp::now(), q.get().raw());
    }
    stop();
    txhdl::netlist::write_vhdl_from_env(&Sampler::lowered("sampler"));
    print!("\n{}", Sampler::verilog("sampler"));
}
