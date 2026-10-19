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

/// Two memories of four bytes, distributed, read and written by a
/// literal index: the byte at `a` of each, xored, and `d` written into
/// the first.
#[derive(Trace, Default)]
pub struct Two {
    #[distributed]
    pub m: Mems<U<8>, 4, 2>,
}

#[lower]
impl Unit<(In<U<2>>, In<U<8>>), Out<U<8>>> for Two {
    async fn run(&mut self, (a, d): (In<U<2>>, In<U<8>>), o: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            o.set(self.m[0].read(a.get()) ^ self.m[1].read(a.get()));
            with!(self <= { m[0].at(a.get()): d.get() });
        }
    }
}

/// A field of `Mems` beside a field of one `Mem` whose name is one of
/// theirs would be, had there been five: the field the array's style
/// could leak to (#1852), refused outright since #1858.
#[derive(Trace, Default)]
pub struct Beside {
    #[ram_style("block")]
    pub way: Mems<U<8>, 4, 4>,
    pub way_4: txhdl::comp::Mem<U<8>, 4>,
}

#[lower]
impl Unit<In<U<2>>, Out<U<8>>> for Beside {
    async fn run(&mut self, a: In<U<2>>, o: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            o.set(self.way[3].read(a.get()) ^ self.way_4.read(a.get()));
        }
    }
}

/// Memories of more than 4096 bits read at three addresses, which no
/// block RAM has the ports for: refused, unless the field says it is
/// distributed, which reaches each memory (issue 1285, #1852).
#[derive(Trace, Default)]
pub struct Big {
    pub m: Mems<U<32>, 256, 2>,
}

#[derive(Trace, Default)]
pub struct BigLut {
    #[distributed]
    pub m: Mems<U<32>, 256, 2>,
}

#[lower]
impl Unit<[In<U<8>>; 3], Out<U<32>>> for Big {
    async fn run(&mut self, a: [In<U<8>>; 3], o: Out<U<32>>) {
        loop {
            DefaultClock::rising().await;
            let x = self.m[1].read(a[0].get()) ^ self.m[1].read(a[1].get());
            o.set(x ^ self.m[1].read(a[2].get()));
        }
    }
}

#[lower]
impl Unit<[In<U<8>>; 3], Out<U<32>>> for BigLut {
    async fn run(&mut self, a: [In<U<8>>; 3], o: Out<U<32>>) {
        loop {
            DefaultClock::rising().await;
            let x = self.m[1].read(a[0].get()) ^ self.m[1].read(a[1].get());
            o.set(x ^ self.m[1].read(a[2].get()));
        }
    }
}

/// `way_4` beside four ways was a field the array's style could leak
/// to (#1852); a field named like an element of an array is refused
/// since #1858, so the unit does not lower at all.
#[test]
fn a_field_beside_an_array_of_its_name_is_refused() {
    let Err(e) = std::panic::catch_unwind(|| Beside::lowered("beside")) else {
        panic!("way_4 beside four ways was not refused");
    };
    let msg = e
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
        .expect("the refusal says why");
    assert!(msg.contains("a field `way_4` beside `way`"), "{msg}");
}

#[test]
fn a_large_memory_of_an_array_is_held_to_a_block_rams_ports() {
    let refused = std::panic::catch_unwind(|| Big::lowered("big"));
    let Err(msg) = refused else {
        panic!("three reads of m_1 were not refused");
    };
    // A panic's payload is a String when formatted, a &str when not.
    let msg = msg
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| msg.downcast_ref::<&str>().map(|s| s.to_string()))
        .expect("the refusal says why");
    assert!(msg.contains("memory `m_1`"), "{msg}");
    let v = BigLut::lowered("big_lut").verilog();
    assert!(v.contains("reg [31:0] m_1 [0:255];"), "{v}");
}

#[test]
fn the_trace_finds_each_memory_by_its_name() {
    let ways = Ways::default();
    let found: Vec<String> = txhdl::comp::trace::memories("ways", &ways)
        .into_iter()
        .map(|(p, _)| p)
        .collect();
    let want: Vec<String> = (0..4).map(|w| format!("ways.way_{w}")).collect();
    assert_eq!(found, want);
    // Each is a memory of its own.
    let cells: std::collections::HashSet<usize> =
        txhdl::comp::trace::memories("ways", &ways)
            .into_iter()
            .map(|(_, c)| c)
            .collect();
    assert_eq!(cells.len(), 4);
}

#[test]
fn a_literal_index_reads_and_writes_in_the_simulation() {
    let (a_o, a_i) = signal::<U<2>, DefaultClock>();
    let (d_o, d_i) = signal::<U<8>, DefaultClock>();
    let (o_o, o_i) = signal::<U<8>, DefaultClock>();
    let mut two = Two::default();
    let mut sim = Running::new(two.run((a_i, d_i), o_o));
    // Write 0x11 * k at each word of m[0], then read them back; m[1]
    // stays zero, so the xor is m[0]'s word.
    for k in 0..4u8 {
        a_o.set(U::from(k));
        d_o.set(U::from(0x11 * k + 1));
        sim.cycle();
    }
    for k in 0..4u8 {
        a_o.set(U::from(k));
        d_o.set(U::from(0x11 * k + 1));
        sim.cycle();
        assert_eq!(o_i.get().raw(), (0x11 * k + 1) as u128, "word {k}");
    }
    let v = Two::lowered("two").verilog();
    assert!(v.contains("m_0["), "{v}");
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

/// A field `r` of one register beside a field `r_0`: no array, so
/// nothing is named twice, and it lowers (#1858).
#[derive(Trace, Default)]
pub struct NotArray {
    pub r: txhdl::comp::Reg<U<8>>,
    pub r_0: txhdl::comp::Reg<U<8>>,
}

#[lower]
impl Unit<In<U<8>>, Out<U<8>>> for NotArray {
    async fn run(&mut self, x: In<U<8>>, o: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            with!(self <= { r: x.get() });
            with!(self <= { r_0: self.r.get() });
            o.set(self.r_0.get());
        }
    }
}

#[test]
fn a_field_beside_a_lone_register_of_its_base_name_lowers() {
    let v = NotArray::lowered("not_array").verilog();
    assert!(v.contains("r_0 <= r;"), "{v}");
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
