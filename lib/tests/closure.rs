// SPDX-License-Identifier: Apache-2.0
//! Local closures in a lowered body, inlined at each call as a
//! `#[lower] fn` would be, and `rx.recv_if(c);` as a statement (issue
//! 1838). Each unit here is lowered beside a twin that says the same
//! without the new form, and the two netlists are the same text.
use txhdl::comp::{Clock, DefaultClock, In, Out, Reg, Rx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

/// A byte-serial header check: whether the byte at position 12 or 13
/// is not the one expected there, with two closures, one of them
/// reading the enclosing body's `p`.
#[derive(Trace, Default)]
pub struct Closures {
    pub seen: Reg<Bit>,
}

#[lower]
impl Unit for Closures {
    async fn run(&mut self, (pos, d): (In<U<11>>, In<U<8>>), miss: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            let p = pos.get();
            let b = d.get();
            let at = |k: U<11>| Bit::from(p == k);
            let ne = |a: U<8>, c: U<8>| Bit::from(a != c);
            let m = (at(U::<11>::from(12u16)) & ne(b, U::<8>::from(0x08u8)))
                | (at(U::<11>::from(13u16)) & ne(b, U::<8>::from(0x00u8)));
            miss.set(m);
            self.seen.set(m);
        }
    }
}

/// The same check written out by hand.
#[derive(Trace, Default)]
pub struct Inline {
    pub seen: Reg<Bit>,
}

#[lower]
impl Unit for Inline {
    async fn run(&mut self, (pos, d): (In<U<11>>, In<U<8>>), miss: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            let p = pos.get();
            let b = d.get();
            let m = (Bit::from(p == U::<11>::from(12u16))
                & Bit::from(b != U::<8>::from(0x08u8)))
                | (Bit::from(p == U::<11>::from(13u16))
                    & Bit::from(b != U::<8>::from(0x00u8)));
            miss.set(m);
            self.seen.set(m);
        }
    }
}

/// The netlist of each, under one name so that the text can be
/// compared.
fn verilog_of(l: txhdl::netlist::Lowered) -> String {
    l.verilog()
}

#[test]
fn a_closure_lowers_as_its_body_written_out() {
    assert_eq!(
        verilog_of(Closures::lowered("unit")),
        verilog_of(Inline::lowered("unit"))
    );
}

/// Takes a word while `take` is high, with the take as a statement.
#[derive(Trace, Default)]
pub struct TakeStmt {
    pub last: Reg<U<8>>,
}

#[lower]
impl Unit for TakeStmt {
    async fn run(
        &mut self,
        (inp, take): (Rx<U<8>>, In<Bit>),
        shown: Out<U<8>>,
    ) {
        loop {
            DefaultClock::rising().await;
            let go = take.get() & inp.peek().is_some();
            let w = inp.head();
            inp.recv_if(go);
            if go.to_bool() {
                self.last.set(w);
            }
            shown.set(self.last.get());
        }
    }
}

/// The same with the binding the statement form saves.
#[derive(Trace, Default)]
pub struct TakeLet {
    pub last: Reg<U<8>>,
}

#[lower]
impl Unit for TakeLet {
    async fn run(
        &mut self,
        (inp, take): (Rx<U<8>>, In<Bit>),
        shown: Out<U<8>>,
    ) {
        loop {
            DefaultClock::rising().await;
            let go = take.get() & inp.peek().is_some();
            let w = inp.head();
            let _ = inp.recv_if(go);
            if go.to_bool() {
                self.last.set(w);
            }
            shown.set(self.last.get());
        }
    }
}

#[test]
fn recv_if_as_a_statement_lowers_as_the_let() {
    assert_eq!(
        verilog_of(TakeStmt::lowered("unit")),
        verilog_of(TakeLet::lowered("unit"))
    );
}

/// A function of the file with the name a closure below takes: the
/// closure, bound later and nearer, is the one called.
#[lower]
fn pick(a: U<8>, _b: U<8>) -> U<8> {
    a
}

/// A closure with `let`s of its own in braces, and one that shadows
/// `pick`.
#[derive(Trace, Default)]
pub struct Braced {
    pub acc: Reg<U<8>>,
}

#[lower]
impl Unit for Braced {
    async fn run(&mut self, (x, y): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let a = x.get();
            let b = y.get();
            let mix = |s: U<8>| {
                let t = s ^ b;
                t + U::<8>::from(1u8)
            };
            let pick = |p: U<8>, q: U<8>| p ^ q;
            let v = pick(mix(a), b);
            self.acc.set(v);
            out.set(self.acc.get());
        }
    }
}

/// The same written out: `pick` is the closure, the two arguments'
/// exclusive or, and not the function, which takes its first.
#[derive(Trace, Default)]
pub struct BracedInline {
    pub acc: Reg<U<8>>,
}

#[lower]
impl Unit for BracedInline {
    async fn run(&mut self, (x, y): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let a = x.get();
            let b = y.get();
            let v = ((a ^ b) + U::<8>::from(1u8)) ^ b;
            self.acc.set(v);
            out.set(self.acc.get());
        }
    }
}

#[test]
fn a_closure_with_lets_and_one_that_shadows_a_function() {
    let _ = pick(U::from(1u8), U::from(2u8));
    assert_eq!(
        verilog_of(Braced::lowered("unit")),
        verilog_of(BracedInline::lowered("unit"))
    );
}

/// A file function that calls `bump`: the file's `bump`, whatever
/// closure of that name the caller has in scope.
#[lower]
fn bump(v: U<8>) -> U<8> {
    v + U::<8>::from(1u8)
}

#[lower]
fn twice(v: U<8>) -> U<8> {
    bump(bump(v))
}

/// The scoping rules, each against a twin written out (agy on PR 1849):
/// a parameter shadows a capture; a later `let` of a captured name
/// leaves the closure's capture as it was; a closure that calls a
/// closure calls the one in scope where it was written; a closure in
/// an inner block ends with it; and a function of the file sees none of
/// its caller's closures.
#[derive(Trace, Default)]
pub struct Scopes {
    pub r: Reg<U<8>>,
}

#[lower]
impl Unit for Scopes {
    async fn run(
        &mut self,
        (x, y, go): (In<U<8>>, In<U<8>>, In<Bit>),
        out: Out<U<8>>,
    ) {
        loop {
            DefaultClock::rising().await;
            let a = x.get();
            let b = y.get();
            // The parameter `a` shadows the captured `a`.
            let p = |a: U<8>| a ^ b;
            // `c` is captured as `a + b`, and later rebinding it does not
            // change the closure.
            let c = a + b;
            let q = |s: U<8>| s & c;
            let c = a - b;
            // `f` is the first `f` inside `g`, though a second follows.
            let f = |s: U<8>| s | U::<8>::from(1u8);
            let g = |s: U<8>| f(s);
            let f = |s: U<8>| s | U::<8>::from(2u8);
            // The caller's `bump` is not the file's, which `twice` calls.
            let bump = |s: U<8>| s ^ U::<8>::from(0x80u8);
            let v = p(a) ^ q(b) ^ c ^ g(a) ^ f(b) ^ twice(a) ^ bump(b);
            if go.get().to_bool() {
                // An inner block's closure, gone after it.
                let h = |s: U<8>| s + U::<8>::from(3u8);
                self.r.set(h(v));
            }
            out.set(self.r.get());
        }
    }
}

#[derive(Trace, Default)]
pub struct ScopesInline {
    pub r: Reg<U<8>>,
}

#[lower]
impl Unit for ScopesInline {
    async fn run(
        &mut self,
        (x, y, go): (In<U<8>>, In<U<8>>, In<Bit>),
        out: Out<U<8>>,
    ) {
        loop {
            DefaultClock::rising().await;
            let a = x.get();
            let b = y.get();
            let c = a + b;
            let c_w = a - b;
            let v = (a ^ b)
                ^ (b & c)
                ^ c_w
                ^ (a | U::<8>::from(1u8))
                ^ (b | U::<8>::from(2u8))
                ^ ((a + U::<8>::from(1u8)) + U::<8>::from(1u8))
                ^ (b ^ U::<8>::from(0x80u8));
            if go.get().to_bool() {
                self.r.set(v + U::<8>::from(3u8));
            }
            out.set(self.r.get());
        }
    }
}

/// The netlist without its comments: the closures' unit says in one
/// that its second `let c` is the wire `c_w`, which the twin names so.
fn uncommented(l: txhdl::netlist::Lowered) -> String {
    l.verilog()
        .lines()
        .filter(|s| !s.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn closures_are_read_where_they_are_written() {
    assert_eq!(
        uncommented(Scopes::lowered("unit")),
        uncommented(ScopesInline::lowered("unit"))
    );
}
