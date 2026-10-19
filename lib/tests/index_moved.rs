// SPDX-License-Identifier: Apache-2.0
//! An array's index moved by a constant (issue 1851): `self.l[j - 1]`
//! and `self.l[j + 1]` in a loop over `j`, read and driven, each folded
//! to a register's name as the loop unrolls; one below zero refused.
use txhdl::comp::{
    mux, signal, Clock, DefaultClock, In, Mems, Out, Regs, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

/// The sixteen largest bytes so far, largest first. The places start
/// at zero, so every byte above zero goes in: after every byte not
/// below it, the ones after it moving along a place, the last falling
/// off. The first, the largest, is put out.
#[derive(Trace, Default)]
pub struct Sorted {
    pub l: Regs<U<8>, 16>,
}

#[lower]
impl Unit<(In<Bit>, In<U<8>>), Out<U<8>>> for Sorted {
    async fn run(&mut self, (go, x): (In<Bit>, In<U<8>>), top: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let v = x.get();
            // Where it goes: after every byte not below it.
            let mut p = U::<5>::from(0u8);
            for j in 0..16 {
                let ahead = Bit::from(self.l[j].get() >= v);
                p = p + mux(ahead, U::<5>::from(1u8), U::<5>::from(0u8));
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
            top.set(self.l[0].get());
        }
    }
}

/// Every byte moves up a place, `l[j + 1]` the target, and the new
/// byte goes in at the bottom.
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

/// Every byte moves down a place, the bottom byte put out and the new
/// byte going in at the top: in four registers, each reading the place
/// above, `l[j + 1]`, and in four memories, each writing the place
/// below, `m[j - 1]`.
#[derive(Trace, Default)]
pub struct Down {
    pub l: Regs<U<8>, 4>,
    pub m: Mems<U<8>, 2, 4>,
}

#[lower]
impl Unit<In<U<8>>, (Out<U<8>>, Out<U<8>>)> for Down {
    async fn run(
        &mut self,
        x: In<U<8>>,
        (bottom, mbottom): (Out<U<8>>, Out<U<8>>),
    ) {
        loop {
            DefaultClock::rising().await;
            // The registers read the place above, `l[j + 1]`.
            for j in 0..3 {
                with!(self <= { l[j]: self.l[j + 1].get() });
            }
            // The memories write the place below, `m[j - 1]`.
            for j in 1..4 {
                let w = self.m[j].read(U::<1>::from(0u8));
                with!(self <= { m[j - 1].at(U::<1>::from(0u8)): w });
            }
            with!(self <= { l[3]: x.get() });
            with!(self <= { m[3].at(U::<1>::from(0u8)): x.get() });
            bottom.set(self.l[0].get());
            mbottom.set(self.m[0].read(U::<1>::from(0u8)));
        }
    }
}

/// The payload of a refusal, as a `String` or a `&str`.
fn said(e: &(dyn std::any::Any + Send)) -> String {
    e.downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
        .expect("the refusal says why")
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
    let (top_o, top_i) = signal::<U<8>, DefaultClock>();
    let mut unit = Sorted::default();
    let regs: Vec<_> = unit.l.0.to_vec();
    let mut sim = Running::new(unit.run((go_i, x_i), top_o));
    let mut model = vec![0u8; 16];
    let mut s = 0x2545_f491u32;
    let mut moved = 0;
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
            let p = model.iter().filter(|&&e| e >= v).count();
            // A byte that goes in before the last place moves the ones
            // after it, through their neighbours.
            if p < 15 && model[p + 1..].iter().any(|&e| e != 0) {
                moved += 1;
            }
            model.insert(p, v);
            model.truncate(16);
        }
        let got: Vec<u8> = regs.iter().map(|r| r.get().raw() as u8).collect();
        assert_eq!(got, model, "step {step}");
        let _ = top_i.get();
    }
    // The run is not the list standing still: bytes went in, and moved
    // others along, many times.
    assert!(model.iter().all(|&e| e != 0), "the list filled: {model:?}");
    assert!(moved > 30, "only {moved} inserts moved a byte");
}

#[test]
fn a_move_down_reads_above_and_drives_below() {
    let (x_o, x_i) = signal::<U<8>, DefaultClock>();
    let (b_o, b_i) = signal::<U<8>, DefaultClock>();
    let (mb_o, mb_i) = signal::<U<8>, DefaultClock>();
    let mut unit = Down::default();
    let mut sim = Running::new(unit.run(x_i, (b_o, mb_o)));
    // Bytes 1 to 8 go in at the top; each reaches the bottom four
    // cycles later, in the registers and in the memories alike.
    let mut seen = vec![];
    for v in 1..=8u8 {
        x_o.set(U::from(v));
        sim.cycle();
        seen.push((b_i.get().raw() as u8, mb_i.get().raw() as u8));
    }
    let want: Vec<(u8, u8)> = (1..=8u8)
        .map(|k| if k <= 4 { (0, 0) } else { (k - 4, k - 4) })
        .collect();
    assert_eq!(seen, want);
    let v = Down::lowered("down").verilog();
    for j in 1..4 {
        let d = format!("l_{} <= l_{j};", j - 1);
        assert!(v.contains(&d), "no `{d}` in:\n{v}");
        assert!(v.contains(&format!("m_{}[", j - 1)), "m_{} written", j - 1);
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
    let msg = said(e.as_ref());
    assert!(msg.contains("self.r[j - 1]"), "{msg}");
    assert!(
        msg.contains("is -1 where the loop's variable is 0"),
        "{msg}"
    );
}

/// An index past the end, three ways: a loop's variable that runs one
/// too far, a neighbour past the last, and a number; and a memory's
/// (#1853). The registers are four and the memories two.
#[derive(Trace, Default)]
pub struct Past {
    pub r: Regs<U<8>, 4>,
    pub m: Mems<U<8>, 2, 2>,
}

#[derive(Trace, Default)]
pub struct PastMoved {
    pub r: Regs<U<8>, 4>,
}

#[derive(Trace, Default)]
pub struct PastNumber {
    pub r: Regs<U<8>, 4>,
}

#[derive(Trace, Default)]
pub struct PastMem {
    pub m: Mems<U<8>, 2, 2>,
}

#[lower]
impl Unit<In<U<8>>, Out<U<8>>> for Past {
    async fn run(&mut self, x: In<U<8>>, o: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let mut s = x.get();
            for j in 0..5 {
                s = s ^ self.r[j].get();
            }
            o.set(s ^ self.m[0].read(U::<1>::from(0u8)));
        }
    }
}

#[lower]
impl Unit<In<U<8>>, Out<U<8>>> for PastMoved {
    async fn run(&mut self, x: In<U<8>>, o: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let mut s = x.get();
            for j in 0..4 {
                s = s ^ self.r[j + 1].get();
            }
            o.set(s);
        }
    }
}

#[lower]
impl Unit<In<U<8>>, Out<U<8>>> for PastNumber {
    async fn run(&mut self, x: In<U<8>>, o: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            o.set(x.get() ^ self.r[4].get());
        }
    }
}

#[lower]
impl Unit<In<U<8>>, Out<U<8>>> for PastMem {
    async fn run(&mut self, x: In<U<8>>, o: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            o.set(x.get() ^ self.m[2].read(U::<1>::from(0u8)));
        }
    }
}

/// Writes past the end: a register driven as `r[4]` of four, and a
/// memory written as `m[2]` of two (#1853).
#[derive(Trace, Default)]
pub struct PastWrite {
    pub r: Regs<U<8>, 4>,
}

#[derive(Trace, Default)]
pub struct PastWriteMem {
    pub m: Mems<U<8>, 2, 2>,
}

#[lower]
impl Unit<In<U<8>>, Out<U<8>>> for PastWrite {
    async fn run(&mut self, x: In<U<8>>, o: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            with!(self <= { r[4]: x.get() });
            o.set(self.r[0].get());
        }
    }
}

#[lower]
impl Unit<In<U<8>>, Out<U<8>>> for PastWriteMem {
    async fn run(&mut self, x: In<U<8>>, o: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            with!(self <= { m[2].at(U::<1>::from(0u8)): x.get() });
            o.set(self.m[0].read(U::<1>::from(0u8)));
        }
    }
}

/// A lone field whose name ends in a number is no array: `stage_2`
/// beside nothing else is read and driven as itself (#1853).
#[derive(Trace, Default)]
pub struct Lone {
    pub stage_2: txhdl::comp::Reg<U<8>>,
}

#[lower]
impl Unit<In<U<8>>, Out<U<8>>> for Lone {
    async fn run(&mut self, x: In<U<8>>, o: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            with!(self <= { stage_2: x.get() });
            o.set(self.stage_2.get());
        }
    }
}

#[test]
fn a_lone_field_ending_in_a_number_lowers() {
    let v = Lone::lowered("lone").verilog();
    assert!(v.contains("stage_2"), "{v}");
}

#[test]
fn an_index_past_the_end_is_refused() {
    let cases: [(fn() -> txhdl::netlist::Lowered, &str); 6] = [
        (|| Past::lowered("past"), "reaches `r_4`"),
        (|| PastMoved::lowered("past"), "reaches `r_4`"),
        (|| PastNumber::lowered("past"), "reaches `r_4`"),
        (|| PastMem::lowered("past"), "reaches `m_2`"),
        (|| PastWrite::lowered("past"), "reaches `r_4`"),
        (|| PastWriteMem::lowered("past"), "reaches `m_2`"),
    ];
    for (k, (lower, want)) in cases.into_iter().enumerate() {
        let Err(e) = std::panic::catch_unwind(lower) else {
            panic!("case {k}: not refused");
        };
        let msg = said(e.as_ref());
        assert!(msg.contains(want), "case {k}: {msg}");
        assert!(msg.contains("past its end"), "case {k}: {msg}");
    }
}

/// The constant beside a loop's variable written every way Rust takes
/// it beside a `usize`: `0_1`, `1usize`, `0x1`, `0o1` and `0b1` are all
/// one, and `1 + j` is `j + 1`, so each place reads the next (issue
/// 1851).
#[derive(Trace, Default)]
pub struct Spellings {
    pub r: Regs<U<8>, 7>,
}

#[lower]
impl Unit<In<U<8>>, Out<U<8>>> for Spellings {
    async fn run(&mut self, x: In<U<8>>, o: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            for j in 0..1 {
                with!(self <= { r[j]: self.r[j + 0_1].get() });
            }
            for j in 1..2 {
                with!(self <= { r[j]: self.r[j + 1usize].get() });
            }
            for j in 2..3 {
                with!(self <= { r[j]: self.r[j + 0x1].get() });
            }
            for j in 3..4 {
                with!(self <= { r[j]: self.r[j + 0o1].get() });
            }
            for j in 4..5 {
                with!(self <= { r[j]: self.r[j + 0b1].get() });
            }
            for j in 5..6 {
                with!(self <= { r[j]: self.r[1 + j].get() });
            }
            with!(self <= { r[6]: x.get() });
            o.set(self.r[0].get());
        }
    }
}

#[test]
fn a_constant_is_one_in_any_spelling() {
    let v = Spellings::lowered("spellings").verilog();
    for j in 0..6 {
        let d = format!("r_{j} <= r_{};", j + 1);
        assert!(v.contains(&d), "no `{d}` in:\n{v}");
    }
}
