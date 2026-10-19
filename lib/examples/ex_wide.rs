// SPDX-License-Identifier: Apache-2.0
//! A value wider than 128 bits: `U<256, 2>` is 256 bits kept in two
//! limbs of 128, the second parameter being the number of limbs, and
//! a `U<N>` of at most 128 bits is `U<N, 1>` as before. The unit is
//! an accumulator at that width: its sum carries across the limbs,
//! and two bytes come out of it as slices. The netlist is checked
//! against the trace under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{signal, Clock, DefaultClock, In, Out, Reg, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

/// Two limbs of 128 bits.
type W = U<256, 2>;
/// One bit past one limb.
type Odd = U<129, 2>;

#[derive(Trace)]
pub struct Acc {
    pub acc: Reg<W>,
    /// Starts past what one limb holds, and counts down.
    pub down: Reg<Odd>,
}

impl Default for Acc {
    fn default() -> Self {
        Acc {
            acc: Reg::default(),
            down: Reg::new(Odd::from_limbs([1, 1])),
        }
    }
}

#[lower]
impl Unit for Acc {
    async fn run(
        &mut self,
        (x, clear): (In<W>, In<Bit>),
        (sum, mid, top, low): (Out<W>, Out<U<8>>, Out<U<8>>, Out<Odd>),
    ) {
        loop {
            DefaultClock::rising().await;
            let (x, clear) = (x.get(), clear.get());
            when!(clear => self { acc: 0 } else { acc: self.acc + x });
            sum.set(self.acc);
            // Two bytes out of the wide value: the one just above the
            // low limb, where a carry lands, and the top one.
            mid.set(self.acc.get().slice::<128, 8>());
            top.set(self.acc.get().slice::<248, 8>());
            // A register's first value at 129 bits, then a borrow
            // out of the high limb on the second cycle.
            low.set(self.down);
            self.down.set(self.down - 1);
        }
    }
}

fn main() {
    let (x_out, x) = signal::<W, DefaultClock>();
    let (clear_out, clear) = signal::<Bit, DefaultClock>();
    let (sum_out, sum) = signal::<W, DefaultClock>();
    let (mid_out, mid) = signal::<U<8>, DefaultClock>();
    let (top_out, top) = signal::<U<8>, DefaultClock>();
    let (low_out, low) = signal::<Odd, DefaultClock>();
    let mut acc = Acc::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("x", &x);
        w.add("clear", &clear);
        w.add("acc", &acc);
        w.add("sum", &sum);
        w.add("mid", &mid);
        w.add("top", &top);
        w.add("low", &low);
        w.start();
    }
    let outs = (sum_out, mid_out, top_out, low_out);
    let mut sim = Running::new(acc.run((x, clear), outs));
    clear_out.set(Bit::One);
    sim.cycle();
    // The counter's first value, 2^128 + 1, which a netlist read into
    // one u128 once lost (#1861).
    let [lo, hi] = low.get().limbs();
    println!("low {hi:x}_{lo:032x}");
    clear_out.set(Bit::Zero);
    // The low limb filled, then one more carries into the high limb;
    // the top bit, then all ones, which is minus one and wraps.
    let ones = u128::MAX;
    let xs: [W; 5] = [
        W::from_limbs([ones, 0]),
        W::from_limbs([1, 0]),
        W::from_limbs([0, 1 << 127]),
        W::from_limbs([ones, ones]),
        W::from_limbs([5, 7]),
    ];
    for v in xs {
        x_out.set(v);
        sim.cycle();
        let [lo, hi] = sum.get().limbs();
        println!("sum {hi:032x}_{lo:032x}  top {:02x}", top.get().raw());
    }
    // The sum is a cycle behind, so the clear shows on the second.
    clear_out.set(Bit::One);
    sim.cycle();
    sim.cycle();
    println!("clear  sum is zero: {}", sum.get() == W::default());
    stop();
    txhdl::netlist::write_vhdl_from_env(&Acc::lowered("acc"));
    print!("\n{}", Acc::verilog("acc"));
}
