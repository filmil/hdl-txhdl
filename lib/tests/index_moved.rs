// SPDX-License-Identifier: Apache-2.0
//! An array's index moved by a constant (issue 1851): `self.l[j - 1]`
//! and `self.l[j + 1]` in a loop over `j`, read and driven, each folded
//! to a register's name as the loop unrolls; one below zero refused.
use txhdl::comp::{
    mux, signal, Clock, DefaultClock, In, Out, Regs, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

/// Sixteen bytes kept in order: a byte in goes where it belongs and
/// the ones above it move up a place, the last falling off. The
/// smallest is put out.
#[derive(Trace, Default)]
pub struct Sorted {
    pub l: Regs<U<8>, 16>,
}

#[lower]
impl Unit<(In<Bit>, In<U<8>>), Out<U<8>>> for Sorted {
    async fn run(&mut self, (go, x): (In<Bit>, In<U<8>>), low: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let v = x.get();
            // Where it goes: after every byte not above it.
            let mut p = U::<5>::from(0u8);
            for j in 0..16 {
                let below = Bit::from(self.l[j].get() <= v);
                p = p + mux(below, U::<5>::from(1u8), U::<5>::from(0u8));
            }
            let at0 = mux(Bit::from(p == 0), v, self.l[0].get());
            with!(self <= { go.get() ? l[0]: at0 });
            for j in 1..16 {
                let keep = Bit::from(p > j);
                let here = Bit::from(p == j);
                let w = mux(
                    keep,
                    self.l[j].get(),
                    mux(here, v, self.l[j - 1].get()),
                );
                with!(self <= { go.get() ? l[j]: w });
            }
            low.set(self.l[0].get());
        }
    }
}

/// The other way round: every byte moves up a place, `l[j + 1]` the
/// target, and the new byte goes in at the bottom.
#[derive(Trace, Default)]
pub struct Shift {
    pub l: Regs<U<8>, 4>,
}

#[lower]
impl Unit<In<U<8>>, Out<U<8>>> for Shift {
    async fn run(&mut self, x: In<U<8>>, top: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            for j in 0..3 {
                with!(self <= { l[j + 1]: self.l[j].get() });
            }
            with!(self <= { l[0]: x.get() });
            top.set(self.l[3].get());
        }
    }
}

/// A neighbour below the first: refused when the loop unrolls.
#[derive(Trace, Default)]
pub struct Below {
    pub r: Regs<U<8>, 4>,
}

#[lower]
impl Unit<In<U<8>>, Out<U<8>>> for Below {
    async fn run(&mut self, x: In<U<8>>, o: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let mut s = x.get();
            for j in 0..4 {
                s = s ^ self.r[j - 1].get();
            }
            o.set(s);
        }
    }
}

#[test]
fn a_sorted_insert_keeps_the_order_in_the_simulation() {
    let (go_o, go_i) = signal::<Bit, DefaultClock>();
    let (x_o, x_i) = signal::<U<8>, DefaultClock>();
    let (low_o, low_i) = signal::<U<8>, DefaultClock>();
    let mut unit = Sorted::default();
    let regs: Vec<_> = unit.l.0.to_vec();
    let mut sim = Running::new(unit.run((go_i, x_i), low_o));
    let mut model = vec![0u8; 16];
    let mut s = 0x2545_f491u32;
    for step in 0..200 {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        let v = (s >> 8) as u8;
        let go = s & 3 != 0;
        go_o.set(Bit::from_bool(go));
        x_o.set(U::from(v));
        sim.cycle();
        if go {
            let p = model.iter().filter(|&&e| e <= v).count();
            model.insert(p, v);
            model.truncate(16);
        }
        let got: Vec<u8> = regs.iter().map(|r| r.get().raw() as u8).collect();
        assert_eq!(got, model, "step {step}");
        let _ = low_i.get();
    }
}

#[test]
fn a_neighbour_is_a_registers_name_in_the_netlist() {
    let v = Sorted::lowered("sorted").verilog();
    // Each place but the first takes its own byte, the new one, or the
    // one below it.
    for j in 1..16 {
        let below = format!("l_{}", j - 1);
        assert!(v.contains(&format!("l_{j} <=")), "l_{j} driven:\n{v}");
        assert!(v.contains(&below), "{below} read:\n{v}");
    }
    let v = Shift::lowered("shift").verilog();
    for j in 0..3 {
        let d = format!("l_{} <= l_{j};", j + 1);
        assert!(v.contains(&d), "no `{d}` in:\n{v}");
    }
}

#[test]
fn a_neighbour_below_the_first_is_refused() {
    let refused = std::panic::catch_unwind(|| Below::lowered("below"));
    let Err(e) = refused else {
        panic!("r[j - 1] at j = 0 was not refused");
    };
    let msg = e.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(msg.contains("self.r[j - 1]"), "{msg}");
    assert!(
        msg.contains("is -1 where the loop's variable is 0"),
        "{msg}"
    );
}
