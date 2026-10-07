// SPDX-License-Identifier: Apache-2.0
//! What the lowering refuses of a memory a block RAM cannot hold
//! (issue 1285). A block RAM has two ports, and a read at the address a
//! write is at shares the write's port. A memory of more than 4096 bits
//! written at two addresses, or reached at three, is refused when it is
//! lowered, naming it; one a block RAM holds is lowered; a small one, or
//! one marked `#[distributed]`, is left alone.
use txhdl::comp::{Clock, DefaultClock, In, Mem, Out, Reg, Unit};
use txhdl::types::U;
use txhdl::{lower, with, Trace};

/// Read at two addresses beside a write at a third: `Dmem` before
/// issue 1301, which read at a new read's and a burst's next and wrote
/// at a held write's.
#[derive(Trace, Default)]
pub struct TwoReads {
    pub m: Mem<U<32>, 1024>,
    pub a: Reg<U<10>>,
    pub b: Reg<U<10>>,
    pub c: Reg<U<10>>,
}

#[lower]
impl Unit for TwoReads {
    async fn run(&mut self, _i: (), out: Out<U<32>>) {
        loop {
            DefaultClock::rising().await;
            out.set(self.m.read(self.a.get()) ^ self.m.read(self.b.get()));
            with!(self <= {
                m.at(self.c.get()): U::<32>::from(1u8),
                c: self.c.get() + 5,
                a: self.a.get() + 1,
                b: self.b.get() + 3,
            });
        }
    }
}

/// Written at two addresses: Razboj's depth bank before issue 992.
#[derive(Trace, Default)]
pub struct TwoWrites {
    pub m: Mem<U<32>, 1024>,
    pub a: Reg<U<10>>,
    pub b: Reg<U<10>>,
    pub t: Reg<U<1>>,
}

#[lower]
impl Unit for TwoWrites {
    async fn run(&mut self, _i: (), out: Out<U<32>>) {
        loop {
            DefaultClock::rising().await;
            out.set(self.m.read(self.a.get()));
            with!(self <= {
                t: !self.t.get(),
                self.t.get() == 0 ? m.at(self.a.get()): U::<32>::from(1u8),
                self.t.get() == 1 ? m.at(self.b.get()): U::<32>::from(2u8),
                a: self.a.get() + 1,
                b: self.b.get() + 3,
            });
        }
    }
}

/// A read-modify-write at one address beside a read at another: two
/// ports, which a block RAM has.
#[derive(Trace, Default)]
pub struct Rmw {
    pub m: Mem<U<32>, 1024>,
    pub a: Reg<U<10>>,
    pub b: Reg<U<10>>,
}

#[lower]
impl Unit for Rmw {
    async fn run(&mut self, _i: (), out: Out<U<32>>) {
        loop {
            DefaultClock::rising().await;
            out.set(self.m.read(self.b.get()));
            with!(self <= {
                m.at(self.a.get()): self.m.read(self.a.get()) + 1,
                a: self.a.get() + 1,
                b: self.b.get() + 3,
            });
        }
    }
}

/// Two reads beside a write, but marked LUT RAM on purpose.
#[derive(Trace, Default)]
pub struct Marked {
    #[distributed]
    pub m: Mem<U<32>, 1024>,
    pub a: Reg<U<10>>,
    pub b: Reg<U<10>>,
}

#[lower]
impl Unit for Marked {
    async fn run(&mut self, _i: (), out: Out<U<32>>) {
        loop {
            DefaultClock::rising().await;
            out.set(self.m.read(self.a.get()) ^ self.m.read(self.b.get()));
            with!(self <= {
                m.at(self.a.get()): U::<32>::from(1u8),
                a: self.a.get() + 1,
                b: self.b.get() + 3,
            });
        }
    }
}

/// A memory that asks for a block RAM, as Razboj's colour bank does
/// where Vivado would otherwise make it LUT RAM (issue 1371).
#[derive(Trace, Default)]
pub struct AsksBlock {
    #[ram_style("block")]
    pub m: Mem<U<32>, 1024>,
    pub a: Reg<U<10>>,
    pub b: Reg<U<10>>,
}

#[lower]
impl Unit for AsksBlock {
    async fn run(&mut self, _i: (), out: Out<U<32>>) {
        loop {
            DefaultClock::rising().await;
            out.set(self.m.read(self.b.get()));
            with!(self <= {
                m.at(self.a.get()): self.m.read(self.a.get()) + 1,
                a: self.a.get() + 1,
                b: self.b.get() + 3,
            });
        }
    }
}

/// Two reads beside a write in 4096 bits: small enough to be LUT RAM.
#[derive(Trace, Default)]
pub struct Small {
    pub m: Mem<U<32>, 128>,
    pub a: Reg<U<7>>,
    pub b: Reg<U<7>>,
}

#[lower]
impl Unit for Small {
    async fn run(&mut self, _i: (), out: Out<U<32>>) {
        loop {
            DefaultClock::rising().await;
            out.set(self.m.read(self.a.get()) ^ self.m.read(self.b.get()));
            with!(self <= {
                m.at(self.a.get()): U::<32>::from(1u8),
                a: self.a.get() + 1,
                b: self.b.get() + 3,
            });
        }
    }
}

#[test]
#[should_panic(expected = "memory `m` of `two_reads`, 1024 words of 32 bits, \
                           is reached at 3 addresses and written at 1")]
fn two_reads_beside_a_write_are_refused() {
    let _ = TwoReads::lowered("two_reads");
}

#[test]
#[should_panic(expected = "is reached at 2 addresses and written at 2")]
fn two_writes_are_refused() {
    let _ = TwoWrites::lowered("two_writes");
}

#[test]
fn a_read_modify_write_beside_a_read_is_lowered() {
    let _ = Rmw::lowered("rmw");
}

#[test]
fn a_memory_marked_distributed_is_lowered() {
    let _ = Marked::lowered("marked");
}

#[test]
fn a_memory_says_what_vivado_should_make_it() {
    let block = AsksBlock::lowered("asks_block");
    let v = block.verilog();
    assert!(
        v.contains("(* ram_style = \"block\" *)\n  reg [31:0] m [0:1023];"),
        "the attribute on the memory's declaration: {v}"
    );
    let h = block.vhdl();
    assert!(h.contains("attribute ram_style : string;"), "{h}");
    assert!(
        h.contains("attribute ram_style of m : signal is \"block\";"),
        "{h}"
    );
    // #[distributed] says the same as ram_style("distributed").
    let v = Marked::lowered("marked").verilog();
    assert!(v.contains("(* ram_style = \"distributed\" *)"), "{v}");
    // A memory that says nothing is as before.
    let v = Rmw::lowered("rmw").verilog();
    assert!(!v.contains("ram_style"), "{v}");
}

/// A register whose logic must not go into DSP slices (issue 1383).
#[derive(Trace, Default)]
pub struct NoDsp {
    #[use_dsp("no")]
    pub p: Reg<U<16>>,
    pub q: Reg<U<16>>,
}

#[lower]
impl Unit for NoDsp {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<16>>) {
        loop {
            DefaultClock::rising().await;
            let (a, b) = (a.get(), b.get());
            with!(self <= {
                p: a.zext::<16>().mul::<16>(b.zext::<16>()),
                q: self.p.get(),
            });
            out.set(self.q.get());
        }
    }
}

#[test]
fn a_register_says_whether_vivado_may_use_dsp_slices() {
    let l = NoDsp::lowered("no_dsp");
    let v = l.verilog();
    assert!(v.contains("(* use_dsp = \"no\" *) reg [15:0] p"), "{v}");
    assert!(!v.contains("use_dsp = \"no\" *) reg [15:0] q"), "{v}");
    let h = l.vhdl();
    assert!(h.contains("attribute use_dsp : string;"), "{h}");
    assert!(
        h.contains("attribute use_dsp of p : signal is \"no\";"),
        "{h}"
    );
}

#[test]
fn a_small_memory_is_lowered() {
    let _ = Small::lowered("small");
}
