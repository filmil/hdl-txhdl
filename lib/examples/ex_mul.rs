// SPDX-License-Identifier: Apache-2.0
//! A multiply-accumulate: `mul::<M>` is a product with its width
//! stated, since a width that is the sum of two others cannot be
//! written in stable Rust, and the lowering makes of it a product at
//! that width in either language. The accumulator sums the products
//! of the pairs it is given, and the netlist is checked against the
//! trace under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{signal, Clock, DefaultClock, In, Out, Reg, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

#[derive(Trace, Default)]
pub struct Mac {
    pub acc: Reg<U<20>>,
}

#[lower]
impl Unit<(In<U<8>>, In<U<8>>, In<Bit>), Out<U<20>>> for Mac {
    async fn run(
        &mut self,
        (a, b, clear): (In<U<8>>, In<U<8>>, In<Bit>),
        sum: Out<U<20>>,
    ) {
        loop {
            DefaultClock::rising().await;
            let (a, b, clear) = (a.get(), b.get(), clear.get());
            // The product at twenty bits, then the sum at twenty bits.
            let product = a.zext::<20>().mul::<20>(b.zext::<20>());
            when!(clear => { self.acc <= 0 } else {
                self.acc <= self.acc + product
            });
            sum.set(self.acc);
        }
    }
}

fn main() {
    let (a_out, a) = signal::<U<8>, DefaultClock>();
    let (b_out, b) = signal::<U<8>, DefaultClock>();
    let (clear_out, clear) = signal::<Bit, DefaultClock>();
    let (sum_out, sum) = signal::<U<20>, DefaultClock>();
    let mut mac = Mac::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("a", &a);
        w.add("b", &b);
        w.add("clear", &clear);
        w.add("mac", &mac);
        w.add("sum", &sum);
        w.start();
    }
    let mut sim = Running::new(mac.run((a, b, clear), sum_out));
    clear_out.set(Bit::One);
    sim.cycle();
    clear_out.set(Bit::Zero);
    // A dot product of two short vectors, then a clear and a product
    // that uses every bit of both operands.
    let pairs: [(u8, u8); 6] =
        [(3, 4), (10, 20), (255, 255), (1, 1), (0, 9), (128, 2)];
    for (x, y) in pairs {
        a_out.set(x);
        b_out.set(y);
        sim.cycle();
        println!("{x:>3} * {y:>3}  sum {}", sum.get().raw());
    }
    clear_out.set(Bit::One);
    sim.cycle();
    println!("clear      sum {}", sum.get().raw());
    stop();
    txhdl::netlist::write_vhdl_from_env(&Mac::lowered("mac"));
    print!("\n{}", Mac::verilog("mac"));
}
