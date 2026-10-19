// SPDX-License-Identifier: Apache-2.0
//! An array of memories, `Mems<T, D, N>` (issue 1845): `N` memories of
//! one shape, named `way_0` onward in the netlist, read and written by a
//! literal or a loop's variable in a lowered body, as `Regs` are, with
//! the field's `#[ram_style]` on every one.
use txhdl::comp::{
    mux, signal, Clock, DefaultClock, In, Mems, Out, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace, Value};

/// A write to one way, and a key to look for in every way at a row.
#[derive(Value, Clone, Copy, Default, PartialEq, Debug)]
pub struct Op {
    /// Write `data` to way `way` at row `row`.
    pub wen: Bit,
    pub way: U<2>,
    pub row: U<3>,
    pub data: U<16>,
    /// The row to look in and the key to look for.
    pub at: U<3>,
    pub key: U<16>,
}

/// Four ways of eight words: a write to one, and a look in all four
/// at once, which says which ways hold the key.
#[derive(Trace, Default)]
pub struct Ways {
    #[ram_style("block")]
    pub way: Mems<U<16>, 8, 4>,
}

#[lower]
impl Unit<In<Op>, Out<U<4>>> for Ways {
    async fn run(&mut self, op: In<Op>, hits: Out<U<4>>) {
        loop {
            DefaultClock::rising().await;
            let o = op.get();
            let mut h = U::<4>::from(0u8);
            for w in 0..4 {
                let hit = Bit::from(self.way[w].read(o.at) == o.key);
                h = h | mux(hit, U::<4>::from(1u8 << w), U::<4>::from(0u8));
                let mine = o.wen & Bit::from(o.way == w);
                with!(self <= {
                    mine ? way[w].at(o.row): o.data,
                });
            }
            hits.set(h);
        }
    }
}

/// The same four ways, distributed, with the literal index.
#[derive(Trace, Default)]
pub struct Two {
    #[distributed]
    pub m: Mems<U<8>, 4, 2>,
}

#[lower]
impl Unit<In<U<2>>, Out<U<8>>> for Two {
    async fn run(&mut self, a: In<U<2>>, o: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            o.set(self.m[0].read(a.get()) ^ self.m[1].read(a.get()));
        }
    }
}

#[test]
fn each_memory_is_declared_with_the_fields_style() {
    let v = Ways::lowered("ways").verilog();
    for w in 0..4 {
        let decl =
            format!("(* ram_style = \"block\" *)\n  reg [15:0] way_{w} [0:7];");
        assert!(v.contains(&decl), "no `{decl}` in:\n{v}");
    }
    assert!(!v.contains("way_4"), "only four ways:\n{v}");
    let v = Two::lowered("two").verilog();
    for k in 0..2 {
        let decl = format!(
            "(* ram_style = \"distributed\" *)\n  reg [7:0] m_{k} [0:3];"
        );
        assert!(v.contains(&decl), "no `{decl}` in:\n{v}");
    }
}

#[test]
fn a_loop_reads_and_writes_each_memory() {
    let v = Ways::lowered("ways").verilog();
    for w in 0..4 {
        assert!(v.contains(&format!("way_{w}[")), "way_{w} unused:\n{v}");
    }
}

/// What the simulation of [`Ways`] says, op by op, against a model.
#[test]
fn the_ways_hold_what_was_written_in_the_simulation() {
    let (op_o, op_i) = signal::<Op, DefaultClock>();
    let (hits_o, hits_i) = signal::<U<4>, DefaultClock>();
    let mut ways = Ways::default();
    let mut sim = Running::new(ways.run(op_i, hits_o));
    let mut model = [[0u16; 8]; 4];
    let mut seed = 0x1234_5678u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for step in 0..400 {
        let r = next();
        // Few keys, so that ways often hold the one looked for.
        let op = Op {
            wen: Bit::from_bool(r & 1 == 1),
            way: U::from((r >> 1) & 3),
            row: U::from((r >> 3) & 7),
            data: U::from((r >> 6) & 3),
            at: U::from((r >> 8) & 7),
            key: U::from((r >> 11) & 3),
        };
        op_o.set(op);
        sim.cycle();
        // The look reads the ways as they were before this op's write.
        let at = op.at.raw() as usize;
        let key = op.key.raw() as u16;
        let mut want = 0u128;
        for (w, rows) in model.iter().enumerate() {
            if rows[at] == key {
                want |= 1 << w;
            }
        }
        if step > 0 {
            assert_eq!(hits_i.get().raw(), want, "step {step}");
        }
        if op.wen.to_bool() {
            model[op.way.raw() as usize][op.row.raw() as usize] =
                op.data.raw() as u16;
        }
    }
}

/// A field named like an element of an array field beside it: `r_3`
/// beside `r`'s four registers, and `way_1` beside two memories, each
/// of which would be declared twice; and `r_4` beside four, past the
/// end, which `self.r[4]` would fold into. Each is refused (#1858).
#[derive(Trace, Default)]
pub struct DupReg {
    pub r: txhdl::comp::Regs<U<8>, 4>,
    pub r_3: txhdl::comp::Reg<U<8>>,
}

#[derive(Trace, Default)]
pub struct DupMem {
    pub way: Mems<U<8>, 4, 2>,
    pub way_1: txhdl::comp::Mem<U<8>, 4>,
}

#[lower]
impl Unit<In<U<8>>, Out<U<8>>> for DupReg {
    async fn run(&mut self, x: In<U<8>>, o: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            o.set(self.r[3].get() ^ self.r_3.get());
            with!(self <= { r_3: x.get() });
        }
    }
}

#[lower]
impl Unit<In<U<2>>, Out<U<8>>> for DupMem {
    async fn run(&mut self, a: In<U<2>>, o: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            o.set(self.way[1].read(a.get()) ^ self.way_1.read(a.get()));
        }
    }
}

#[derive(Trace, Default)]
pub struct Beyond {
    pub r: txhdl::comp::Regs<U<8>, 4>,
    pub r_4: txhdl::comp::Reg<U<8>>,
}

#[lower]
impl Unit<In<U<8>>, Out<U<8>>> for Beyond {
    async fn run(&mut self, x: In<U<8>>, o: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            o.set(self.r[0].get() ^ self.r_4.get());
            with!(self <= { r_4: x.get() });
        }
    }
}

#[test]
fn a_field_named_like_an_element_is_refused() {
    let cases: [(fn() -> txhdl::netlist::Lowered, &str); 3] = [
        (|| DupReg::lowered("dup_reg"), "a field `r_3` beside `r`"),
        (
            || DupMem::lowered("dup_mem"),
            "a field `way_1` beside `way`",
        ),
        (|| Beyond::lowered("beyond"), "a field `r_4` beside `r`"),
    ];
    for (k, (lower, want)) in cases.into_iter().enumerate() {
        let Err(e) = std::panic::catch_unwind(lower) else {
            panic!("case {k}: not refused");
        };
        let msg = e
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
            .expect("the refusal says why");
        assert!(msg.contains(want), "case {k}: {msg}");
    }
}
