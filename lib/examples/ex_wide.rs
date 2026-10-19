// SPDX-License-Identifier: Apache-2.0
//! A value wider than 128 bits: `U<256, 2>` is 256 bits kept in two
//! limbs of 128, the second parameter being the number of limbs, and
//! a `U<N>` of at most 128 bits is `U<N, 1>` as before. The unit is
//! an accumulator at that width: its sum carries across the limbs,
//! and two bytes come out of it as slices. The wide forms take wider
//! pieces: a 200-bit field, the low and top 150 bits joined the other
//! way round, a sign extension, a zero extension and a truncation, and
//! the 258-bit square of a 129-bit value. The netlist is checked
//! against the trace under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{signal, Clock, DefaultClock, In, Out, Reg, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

/// Two limbs of 128 bits.
type W = U<256, 2>;
/// One bit past one limb.
type Odd = U<129, 2>;
/// A field of the sum, and what is made of it, wider than one limb.
type F = U<200, 2>;
type T = U<300, 3>;
/// The square of a 129-bit value.
type Sq = U<258, 3>;

#[derive(Trace)]
pub struct Acc {
    /// Starts at a value no single limb holds.
    pub acc: Reg<W>,
    /// Starts past what one limb holds, and counts down.
    pub down: Reg<Odd>,
}

impl Default for Acc {
    fn default() -> Self {
        Acc {
            acc: Reg::new(W::from_limbs([7, 1])),
            down: Reg::new(Odd::from_limbs([1, 1])),
        }
    }
}

#[lower]
impl Unit for Acc {
    async fn run(
        &mut self,
        (x, clear): (In<W>, In<Bit>),
        (sum, mid, top, low, field, swapped, ext, grown, cut, square): (
            Out<W>,
            Out<U<8>>,
            Out<U<8>>,
            Out<Odd>,
            Out<F>,
            Out<T>,
            Out<T>,
            Out<T>,
            Out<Odd>,
            Out<Sq>,
        ),
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
            // Wider pieces, which state their limbs as well: 200 bits
            // from bit 28, and the low 150 bits above the top 150,
            // which share the middle 44.
            field.set(self.acc.get().slice_wide::<28, 200, 2>());
            let hi = self.acc.get().slice_wide::<106, 150, 2>();
            let lo = self.acc.get().slice_wide::<0, 150, 2>();
            swapped.set(lo.concat_wide::<150, 2, 300, 3>(hi));
            // Wider and narrower again, still in limbs.
            grown.set(self.acc.get().zext_wide::<300, 3>());
            cut.set(self.acc.get().resize_wide::<129, 2>());
            // A register's first value at 129 bits, then a borrow
            // out of the high limb on the second cycle; its top bit
            // is set at first, so the extension fills with ones.
            low.set(self.down);
            ext.set(self.down.get().sext_wide::<300, 3>());
            square.set(self.down.get().mul_wide::<258, 3>(self.down.get()));
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
    let (field_out, field) = signal::<F, DefaultClock>();
    let (swapped_out, swapped) = signal::<T, DefaultClock>();
    let (ext_out, ext) = signal::<T, DefaultClock>();
    let (grown_out, grown) = signal::<T, DefaultClock>();
    let (cut_out, cut) = signal::<Odd, DefaultClock>();
    let (square_out, square) = signal::<Sq, DefaultClock>();
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
        w.add("field", &field);
        w.add("swapped", &swapped);
        w.add("ext", &ext);
        w.add("grown", &grown);
        w.add("cut", &cut);
        w.add("square", &square);
        w.start();
    }
    let outs = (
        sum_out,
        mid_out,
        top_out,
        low_out,
        field_out,
        swapped_out,
        ext_out,
        grown_out,
        cut_out,
        square_out,
    );
    let mut sim = Running::new(acc.run((x, clear), outs));
    clear_out.set(Bit::One);
    sim.cycle();
    // The first values, 2^128 + 7 and 2^128 + 1, which a netlist
    // read into one u128 once lost (#1861).
    let [lo, hi] = sum.get().limbs();
    println!("sum {hi:032x}_{lo:032x}");
    let [lo, hi] = low.get().limbs();
    println!("low {hi:x}_{lo:032x}");
    let [_, _, s] = ext.get().limbs();
    println!("ext, top limb {s:x}");
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
