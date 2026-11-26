// SPDX-License-Identifier: Apache-2.0
//! A configuration constant in a lowered body (issues 2069 and 2083).
//! A `bool` constant lowers as a one-bit literal, by its type, so its
//! negation is one bit wide in Verilog wherever it is written; VHDL
//! gives a comparison of two literals no type, and nvc refused
//! `out_rw <= a when ('1' = '1') else b;` as an ambiguous use of `'1'`.
//! The operand not chosen is an input port here, and the port stays on
//! the unit, read by nothing.
use std::marker::PhantomData;
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::netlist::{Expr, Lowered, Stmt};
use txhdl::types::{Bit, U};
use txhdl::{check, lower, Trace};

pub trait PickConfig: Default {
    const FLAG: bool;
}

#[derive(Default)]
pub struct Yes;
impl PickConfig for Yes {
    const FLAG: bool = true;
}

#[derive(Default)]
pub struct No;
impl PickConfig for No {
    const FLAG: bool = false;
}

/// `mux` on the constant.
#[derive(Default, Trace)]
pub struct Pick<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for Pick<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            out.set(mux(Bit::from(C::FLAG), a.get(), b.get()));
        }
    }
}

/// An `if` that yields a value, on the constant.
#[derive(Default, Trace)]
pub struct PickIf<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickIf<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let v = if Bit::from(C::FLAG).to_bool() {
                a.get()
            } else {
                b.get()
            };
            out.set(v);
        }
    }
}

/// The constant and a literal bit, written inline as the selector.
#[derive(Default, Trace)]
pub struct PickAnd<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickAnd<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            out.set(mux(Bit::from(C::FLAG) & Bit::One, a.get(), b.get()));
        }
    }
}

/// A bare `if` on the constant, as a configuration is usually read.
#[derive(Default, Trace)]
pub struct PickBare<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickBare<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let v = if C::FLAG { a.get() } else { b.get() };
            out.set(v);
        }
    }
}

/// A `match` that yields a value, on the constant.
#[derive(Default, Trace)]
pub struct PickMatch<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickMatch<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let v = match Bit::from(C::FLAG) {
                Bit::One => a.get(),
                _ => b.get(),
            };
            out.set(v);
        }
    }
}

/// The constant compared with a literal bit, as the selector.
#[derive(Default, Trace)]
pub struct PickEq<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickEq<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            out.set(mux(Bit::from(C::FLAG) == Bit::One, a.get(), b.get()));
        }
    }
}

/// A bare number as the operand the constant chooses: the choice is
/// kept, since the number has no width of its own.
#[derive(Default, Trace)]
pub struct PickNum<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickNum<C> {
    async fn run(&mut self, (a, _b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            out.set(mux(Bit::from(C::FLAG), U::from(0u8), a.get()));
        }
    }
}

/// A check on the constant and a signal: it reaches the VHDL of a
/// condition with a constant in it, and no fold before.
#[derive(Default, Trace)]
pub struct CheckConst<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for CheckConst<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            check!(
                Bit::from(C::FLAG) == Bit::One || a.get() == b.get(),
                "the flag is set, or the two agree"
            );
            out.set(a.get());
        }
    }
}

/// The negation of the constant as the selector: a `bool` constant is
/// a one-bit literal, so Verilog writes `~1'b1`, false, where it wrote
/// `~1`, thirty-two bits and true (issue 2083).
#[derive(Default, Trace)]
pub struct PickNot<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickNot<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            out.set(mux(!Bit::from(C::FLAG), a.get(), b.get()));
        }
    }
}

/// The negation inside the conversion, `Bit::from(!C::FLAG)`.
#[derive(Default, Trace)]
pub struct PickNotIn<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickNotIn<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            out.set(mux(Bit::from(!C::FLAG), a.get(), b.get()));
        }
    }
}

/// The negated constant as the selector, with no conversion written.
#[derive(Default, Trace)]
pub struct PickNotBare<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickNotBare<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            out.set(mux(!C::FLAG, a.get(), b.get()));
        }
    }
}

/// An `if` that yields a value, on the negated constant.
#[derive(Default, Trace)]
pub struct PickIfNot<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickIfNot<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let v = if !C::FLAG { a.get() } else { b.get() };
            out.set(v);
        }
    }
}

/// The negated constant or a signal, as the selector.
#[derive(Default, Trace)]
pub struct PickNotOr<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickNotOr<C> {
    async fn run(
        &mut self,
        (a, b, x): (In<U<8>>, In<U<8>>, In<Bit>),
        out: Out<U<8>>,
    ) {
        loop {
            DefaultClock::rising().await;
            out.set(mux(!Bit::from(C::FLAG) | x.get(), a.get(), b.get()));
        }
    }
}

/// An `if` statement on the negated constant, driving a register.
#[derive(Default, Trace)]
pub struct PickNotReg<C: PickConfig> {
    r: Reg<U<8>>,
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickNotReg<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            out.set(self.r.get());
            if !C::FLAG {
                self.r.set(a.get());
            } else {
                self.r.set(b.get());
            }
        }
    }
}

/// The text of every unit, in both languages, for the test that reads
/// it, and printed when a test fails.
fn both<U: Fn(&str) -> txhdl::netlist::Lowered>(f: U) -> (String, String) {
    let l = f("p");
    (l.vhdl(), l.verilog())
}

/// The conditions of a VHDL text: what `when` chooses on, what `if`
/// tests and what `assert` states.
fn conditions(h: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in h.lines().map(str::trim) {
        if line.starts_with("begin if c then") {
            continue;
        }
        if let Some((_, rest)) = line.split_once(" when ") {
            if let Some((c, _)) = rest.rsplit_once(" else ") {
                out.push(c.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("if ") {
            if let Some(c) = rest.strip_suffix(" then") {
                out.push(c.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("assert ") {
            if let Some((c, _)) = rest.split_once(" report ") {
                out.push(c.to_string());
            }
        }
    }
    out
}

/// Whether a condition names a signal: any word left once the words
/// of VHDL itself and the qualified literals are taken out.
fn names_a_signal(c: &str) -> bool {
    let c = c
        .replace("std_ulogic'('1')", "")
        .replace("std_ulogic'('0')", "");
    let c = c.replace("'1'", "").replace("'0'", "");
    c.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .filter(|w| !w.is_empty())
        .filter(|w| !w.chars().next().is_some_and(|ch| ch.is_ascii_digit()))
        .any(|w| {
            !matches!(
                w,
                "not" | "and" | "or" | "xor" | "true" | "false" | "mux"
            )
        })
}

/// Legal VHDL for a constant condition: a condition that names no
/// signal writes every one-bit literal qualified, since nothing else
/// types it, and nvc refused `('1' = '1')` as ambiguous (issue 2069).
fn legal(h: &str) {
    for c in conditions(h) {
        if names_a_signal(&c) {
            continue;
        }
        let bare = c
            .replace("std_ulogic'('1')", "")
            .replace("std_ulogic'('0')", "");
        assert!(
            !bare.contains("'1'") && !bare.contains("'0'"),
            "an unqualified literal in `{c}`, which names no signal: {h}"
        );
    }
}

/// A `mux` and an `if` on the bare constant fold to the operand they
/// choose, in both languages, and the port not chosen stays.
#[test]
fn a_choice_on_the_constant_is_a_wire() {
    let (h, v) = both(Pick::<Yes>::lowered);
    assert!(
        h.contains("out_rw <= a;") && v.contains("assign out_rw = a;"),
        "{h}{v}"
    );
    assert!(
        h.contains("b : in unsigned(7 downto 0)")
            && v.contains("input [7:0] b")
    );
    let (h, v) = both(Pick::<No>::lowered);
    assert!(
        h.contains("out_rw <= b;") && v.contains("assign out_rw = b;"),
        "{h}{v}"
    );
    for (h, v, x) in [
        (both(PickIf::<Yes>::lowered), "a"),
        (both(PickIf::<No>::lowered), "b"),
        (both(PickBare::<Yes>::lowered), "a"),
        (both(PickBare::<No>::lowered), "b"),
    ]
    .map(|((h, v), x)| (h, v, x))
    {
        assert!(h.contains(&format!("v <= {x};")), "{h}");
        assert!(v.contains(&format!("assign v = {x};")), "{v}");
    }
}

/// A selector worked out of constants stays a choice, and its VHDL is
/// legal and typed: `&`, `==` and a `match` on the constant. The
/// Verilog keeps the choice, on a sized selector.
#[test]
fn a_selector_worked_out_of_constants_stays_a_legal_choice() {
    let q1 = "std_ulogic'('1')";
    for (h, v, sel) in [
        (both(PickAnd::<Yes>::lowered), "(1'b1 & 1'b1) ?"),
        (both(PickAnd::<No>::lowered), "(1'b0 & 1'b1) ?"),
        (both(PickEq::<Yes>::lowered), "(1'b1 == 1'b1) ?"),
        (both(PickEq::<No>::lowered), "(1'b0 == 1'b1) ?"),
    ]
    .map(|((h, v), s)| (h, v, s))
    {
        legal(&h);
        assert!(h.contains(q1), "{h}");
        assert!(v.contains(sel), "{sel} in {v}");
    }
    for (h, v) in [
        both(PickMatch::<Yes>::lowered),
        both(PickMatch::<No>::lowered),
    ] {
        legal(&h);
        assert!(h.contains(q1), "{h}");
        // The selector compares sized bits, never a bare number.
        assert!(v.contains(" ? ") && v.contains("1'b"), "{v}");
        for bad in ["(1 ", "(0 ", " 1)", " 0)"] {
            assert!(!v.contains(bad), "{bad} in {v}");
        }
    }
}

/// The negation of the constant chooses as the run does, in both
/// languages: the selector is a one-bit literal, not a number.
#[test]
fn the_negation_of_the_constant_chooses_as_the_run_does() {
    let (h, v) = both(PickNot::<Yes>::lowered);
    legal(&h);
    assert!(h.contains("out_rw <= a when (not true) else b;"), "{h}");
    assert!(v.contains("assign out_rw = ((~1'b1) ? a : b);"), "{v}");
    let (h, v) = both(PickNot::<No>::lowered);
    assert!(h.contains("out_rw <= a when (not false) else b;"), "{h}");
    assert!(v.contains("assign out_rw = ((~1'b0) ? a : b);"), "{v}");
}

/// A bare number chosen by the constant keeps its choice, written with
/// the constant as a truth value; where the constant chooses the other
/// operand, that one is the wire.
#[test]
fn a_bare_number_keeps_its_choice() {
    let (h, v) = both(PickNum::<Yes>::lowered);
    legal(&h);
    assert!(h.contains("when true else"), "{h}");
    assert!(v.contains("(1'b1 ? "), "{v}");
    let (h, v) = both(PickNum::<No>::lowered);
    legal(&h);
    assert!(h.contains("out_rw <= a;"), "{h}");
    assert!(v.contains("assign out_rw = a;"), "{v}");
}

/// A check with a constant in its condition is legal and typed in VHDL
/// and holds the sized constant in Verilog, for both values.
#[test]
fn a_check_on_a_constant_is_legal() {
    for (h, v, bit) in [
        (both(CheckConst::<Yes>::lowered), "1'b1"),
        (both(CheckConst::<No>::lowered), "1'b0"),
    ]
    .map(|((h, v), b)| (h, v, b))
    {
        legal(&h);
        assert!(h.contains("std_ulogic'('"), "{h}");
        assert!(v.contains(&format!("{bit} == 1'b1")), "{v}");
    }
}

/// Every other form of the negated constant writes a one-bit literal
/// in Verilog, never `~1` or `~0`, which Verilog widens to thirty-two
/// bits and reads as true (issue 2083); the VHDL is legal.
#[test]
fn every_negation_of_the_constant_is_one_bit() {
    for (h, v) in [
        both(PickNotIn::<Yes>::lowered),
        both(PickNotIn::<No>::lowered),
        both(PickNotBare::<Yes>::lowered),
        both(PickNotBare::<No>::lowered),
        both(PickIfNot::<Yes>::lowered),
        both(PickIfNot::<No>::lowered),
        both(PickNotOr::<Yes>::lowered),
        both(PickNotOr::<No>::lowered),
        both(PickNotReg::<Yes>::lowered),
        both(PickNotReg::<No>::lowered),
    ] {
        legal(&h);
        for bad in ["~1)", "~0)", "~1 ", "~0 ", "!1", "!0"] {
            assert!(!v.contains(bad), "{bad} in {v}");
        }
    }
    let (_, v) = both(PickNotIn::<Yes>::lowered);
    assert!(v.contains("assign out_rw = ((~1'b1) ? a : b);"), "{v}");
    let (_, v) = both(PickNotOr::<Yes>::lowered);
    assert!(v.contains("((~1'b1) | x) ? a : b"), "{v}");
}

/// The negated constant compared with a truth value: `==` on two
/// truth values, which Verilog sized by the wider side.
#[derive(Default, Trace)]
pub struct PickNotEq<C: PickConfig> {
    _c: PhantomData<C>,
}

// The comparison is the form under test, so it is written as a design
// might write it rather than as Clippy would simplify it.
#[allow(clippy::bool_comparison, clippy::nonminimal_bool)]
#[lower]
impl<C: PickConfig> Unit for PickNotEq<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            out.set(mux(!C::FLAG == false, a.get(), b.get()));
        }
    }
}

/// The negated constant or a signal read as a truth value.
#[derive(Default, Trace)]
pub struct PickNotOrBool<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickNotOrBool<C> {
    async fn run(
        &mut self,
        (a, b, x): (In<U<8>>, In<U<8>>, In<Bit>),
        out: Out<U<8>>,
    ) {
        loop {
            DefaultClock::rising().await;
            out.set(mux(!C::FLAG | x.get().to_bool(), a.get(), b.get()));
        }
    }
}

/// The negated constant as a branch of a choice that selects.
#[derive(Default, Trace)]
pub struct PickNested<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickNested<C> {
    async fn run(
        &mut self,
        (a, b, s): (In<U<8>>, In<U<8>>, In<Bit>),
        out: Out<U<8>>,
    ) {
        loop {
            DefaultClock::rising().await;
            let c = if s.get().to_bool() { !C::FLAG } else { false };
            out.set(mux(c, a.get(), b.get()));
        }
    }
}

/// The negated constant under `&&`, bound by `let` before it selects.
#[derive(Default, Trace)]
pub struct PickLetAnd<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickLetAnd<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let p = !C::FLAG && a.get() == b.get();
            out.set(mux(p, a.get(), b.get()));
        }
    }
}

/// A `match` on the negated constant.
#[derive(Default, Trace)]
pub struct PickMatchNot<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickMatchNot<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let v = match !C::FLAG {
                false => a.get(),
                _ => b.get(),
            };
            out.set(v);
        }
    }
}

/// A check on the negated constant or a signal.
#[derive(Default, Trace)]
pub struct CheckNot<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for CheckNot<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            check!(!C::FLAG || a.get() == b.get(), "set, or the two agree");
            out.set(a.get());
        }
    }
}

/// A const generic `bool`, beside the associated constant.
#[derive(Default, Trace)]
pub struct PickGen<const F: bool> {}

#[lower]
impl<const F: bool> Unit for PickGen<F> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            out.set(mux(!F, a.get(), b.get()));
        }
    }
}

/// The constant cast to an integer: a number, as it was before a
/// `bool` constant lowered as a one-bit literal, in a shift, a
/// comparison, an index and a sum.
#[derive(Default, Trace)]
pub struct CastUse<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for CastUse<C> {
    async fn run(
        &mut self,
        a: In<U<8>>,
        (s, e, i, p): (Out<U<8>>, Out<Bit>, Out<Bit>, Out<U<8>>),
    ) {
        loop {
            DefaultClock::rising().await;
            s.set(a.get() << (C::FLAG as u8));
            e.set(Bit::from(a.get() == (C::FLAG as u8)));
            i.set(a.get().bit(C::FLAG as usize));
            p.set(a.get() + U::from(C::FLAG as u8));
        }
    }
}

/// Constants taken from a constant array and a tuple in one: a `bool`
/// item is a truth value, an integer item beside it a number.
pub trait ArrConfig: Default {
    const FLAGS: [bool; 2];
    const PAIRS: [(bool, u8); 1];
}

#[derive(Default)]
pub struct Arr;
impl ArrConfig for Arr {
    const FLAGS: [bool; 2] = [false, true];
    const PAIRS: [(bool, u8); 1] = [(true, 3)];
}

/// The items as selectors and in a comparison.
#[derive(Default, Trace)]
pub struct PickArr<C: ArrConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: ArrConfig> Unit for PickArr<C> {
    async fn run(
        &mut self,
        (a, b): (In<U<8>>, In<U<8>>),
        (x, y, z): (Out<U<8>>, Out<U<8>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            x.set(mux(!C::FLAGS[1], a.get(), b.get()));
            y.set(mux(!C::PAIRS[0].0, a.get(), b.get()));
            z.set(Bit::from(a.get() == C::PAIRS[0].1));
        }
    }
}

/// A positive `if` statement on the constant, driving a register.
#[derive(Default, Trace)]
pub struct PickReg<C: PickConfig> {
    r: Reg<U<8>>,
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickReg<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            out.set(self.r.get());
            if C::FLAG {
                self.r.set(a.get());
            } else {
                self.r.set(b.get());
            }
        }
    }
}

/// The constant compared with a signal by `!=`.
#[derive(Default, Trace)]
pub struct PickNe<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickNe<C> {
    async fn run(
        &mut self,
        (a, b, x): (In<U<8>>, In<U<8>>, In<Bit>),
        out: Out<U<8>>,
    ) {
        loop {
            DefaultClock::rising().await;
            out.set(mux(C::FLAG != x.get().to_bool(), a.get(), b.get()));
        }
    }
}

/// The constant bound by `let`, then negated: an alias of a constant,
/// which the lowering keeps apart from a loop's index.
#[derive(Default, Trace)]
pub struct PickAlias<C: PickConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: PickConfig> Unit for PickAlias<C> {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let k = C::FLAG;
            out.set(mux(!k, a.get(), b.get()));
        }
    }
}

/// A register whose reset value is the constant.
#[derive(Trace)]
pub struct PickRst<C: PickConfig> {
    r: Reg<Bit>,
    _c: PhantomData<C>,
}

impl<C: PickConfig> Default for PickRst<C> {
    fn default() -> Self {
        Self {
            r: Reg::new(Bit::from(C::FLAG)),
            _c: PhantomData,
        }
    }
}

#[lower]
impl<C: PickConfig> Unit for PickRst<C> {
    async fn run(&mut self, x: In<Bit>, out: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            out.set(self.r.get());
            self.r.set(self.r.get() ^ x.get());
        }
    }
}

/// `legal` itself: it refuses the line nvc refused.
#[test]
#[should_panic(expected = "an unqualified literal")]
fn legal_refuses_the_line_nvc_refused() {
    legal("  out_rw <= a when ('1' = '1') else b;");
}

/// The text each form writes, in both languages, for both values: the
/// VHDL line, then the Verilog line. Each was checked by hand against
/// the run's choice.
#[test]
fn every_form_writes_the_text_that_chooses_as_the_run_does() {
    let q = |b: &str| format!("std_ulogic'('{b}')");
    let cases: Vec<(&str, Lowered, String, String)> = vec![
        (
            "PickAnd<Yes>",
            PickAnd::<Yes>::lowered("p"),
            format!(
                "out_rw <= a when (({} and {}) = {}) else b;",
                q("1"),
                q("1"),
                q("1")
            ),
            "assign out_rw = ((1'b1 & 1'b1) ? a : b);".into(),
        ),
        (
            "PickAnd<No>",
            PickAnd::<No>::lowered("p"),
            format!(
                "out_rw <= a when (({} and {}) = {}) else b;",
                q("0"),
                q("1"),
                q("1")
            ),
            "assign out_rw = ((1'b0 & 1'b1) ? a : b);".into(),
        ),
        (
            "PickMatch<Yes>",
            PickMatch::<Yes>::lowered("p"),
            format!("v <= a when ({} = {}) else b;", q("1"), q("1")),
            "assign v = ((1'b1 == 1'b1) ? a : b);".into(),
        ),
        (
            "PickMatch<No>",
            PickMatch::<No>::lowered("p"),
            format!("v <= a when ({} = {}) else b;", q("0"), q("1")),
            "assign v = ((1'b0 == 1'b1) ? a : b);".into(),
        ),
        (
            "PickNotIn<Yes>",
            PickNotIn::<Yes>::lowered("p"),
            "out_rw <= a when (not true) else b;".into(),
            "assign out_rw = ((~1'b1) ? a : b);".into(),
        ),
        (
            "PickNotIn<No>",
            PickNotIn::<No>::lowered("p"),
            "out_rw <= a when (not false) else b;".into(),
            "assign out_rw = ((~1'b0) ? a : b);".into(),
        ),
        (
            "PickNotBare<Yes>",
            PickNotBare::<Yes>::lowered("p"),
            "out_rw <= a when (not true) else b;".into(),
            "assign out_rw = ((~1'b1) ? a : b);".into(),
        ),
        (
            "PickNotBare<No>",
            PickNotBare::<No>::lowered("p"),
            "out_rw <= a when (not false) else b;".into(),
            "assign out_rw = ((~1'b0) ? a : b);".into(),
        ),
        (
            "PickIfNot<Yes>",
            PickIfNot::<Yes>::lowered("p"),
            "v <= a when (not true) else b;".into(),
            "assign v = ((~1'b1) ? a : b);".into(),
        ),
        (
            "PickIfNot<No>",
            PickIfNot::<No>::lowered("p"),
            "v <= a when (not false) else b;".into(),
            "assign v = ((~1'b0) ? a : b);".into(),
        ),
        (
            "PickNotOr<Yes>",
            PickNotOr::<Yes>::lowered("p"),
            "out_rw <= a when (((not '1') or x) = '1') else b;".into(),
            "assign out_rw = (((~1'b1) | x) ? a : b);".into(),
        ),
        (
            "PickNotOrBool<No>",
            PickNotOrBool::<No>::lowered("p"),
            "out_rw <= a when (((not '0') or x) = '1') else b;".into(),
            "assign out_rw = (((~1'b0) | x) ? a : b);".into(),
        ),
        (
            "PickNotReg<Yes>",
            PickNotReg::<Yes>::lowered("p"),
            "if (not true) then".into(),
            "if ((~1'b1)) begin".into(),
        ),
        (
            "PickNotReg<No>",
            PickNotReg::<No>::lowered("p"),
            "if (not false) then".into(),
            "if ((~1'b0)) begin".into(),
        ),
        (
            "PickNotEq<Yes>",
            PickNotEq::<Yes>::lowered("p"),
            format!("out_rw <= a when ((not {}) = {}) else b;", q("1"), q("0")),
            "assign out_rw = (((~1'b1) == 1'b0) ? a : b);".into(),
        ),
        (
            "PickNotEq<No>",
            PickNotEq::<No>::lowered("p"),
            format!("out_rw <= a when ((not {}) = {}) else b;", q("0"), q("0")),
            "assign out_rw = (((~1'b0) == 1'b0) ? a : b);".into(),
        ),
        (
            "PickNested<Yes>",
            PickNested::<Yes>::lowered("p"),
            "c <= (not '1') when (s = '1') else '0';".into(),
            "assign c = (s ? (~1'b1) : 1'b0);".into(),
        ),
        (
            "PickLetAnd<Yes>",
            PickLetAnd::<Yes>::lowered("p"),
            "p <= '1' when ((not true) and (a = b)) else '0';".into(),
            "assign p = ((~1'b1) && (a == b));".into(),
        ),
        (
            "PickMatchNot<Yes>",
            PickMatchNot::<Yes>::lowered("p"),
            format!("v <= a when ((not {}) = {}) else b;", q("1"), q("0")),
            "assign v = (((~1'b1) == 1'b0) ? a : b);".into(),
        ),
        (
            "CheckNot<Yes>",
            CheckNot::<Yes>::lowered("p"),
            "assert rst = '1' or (((not true) or (a = b))) report".into(),
            "if (!rst) assert (((~1'b1) || (a == b)));".into(),
        ),
        (
            "PickNe<Yes>",
            PickNe::<Yes>::lowered("p"),
            "out_rw <= a when ('1' /= x) else b;".into(),
            "assign out_rw = ((1'b1 != x) ? a : b);".into(),
        ),
        (
            "PickAlias<Yes>",
            PickAlias::<Yes>::lowered("p"),
            "out_rw <= a when (not true) else b;".into(),
            "assign out_rw = ((~1'b1) ? a : b);".into(),
        ),
        (
            "PickAlias<No>",
            PickAlias::<No>::lowered("p"),
            "out_rw <= a when (not false) else b;".into(),
            "assign out_rw = ((~1'b0) ? a : b);".into(),
        ),
    ];
    for (name, l, hl, vl) in cases {
        let (h, v) = (l.vhdl(), l.verilog());
        assert!(h.contains(&hl), "{name}: {hl} in {h}");
        assert!(v.contains(&vl), "{name}: {vl} in {v}");
    }
}

/// The constant cast to an integer is the number it stands for, as
/// it was before a `bool` constant lowered as a one-bit literal: the
/// same text in a shift, a comparison, an index and a sum.
#[test]
fn a_cast_of_the_constant_is_the_number_it_was() {
    for (l, k) in [
        (CastUse::<Yes>::lowered("p"), 1),
        (CastUse::<No>::lowered("p"), 0),
    ] {
        let (h, v) = (l.vhdl(), l.verilog());
        for line in [
            format!("s <= shift_left(a, {k});"),
            format!("e <= '1' when (a = to_unsigned({k}, 8)) else '0';"),
            format!("i <= a({k});"),
            format!("p <= (a + to_unsigned({k}, 8));"),
        ] {
            assert!(h.contains(&line), "{line} in {h}");
        }
        for line in [
            format!("assign s = (a << 8'h{k});"),
            format!("assign e = (a == 8'h{k});"),
            format!("assign i = a[{k}];"),
            format!("assign p = (a + 8'h{k});"),
        ] {
            assert!(v.contains(&line), "{line} in {v}");
        }
        assert!(!v.contains("1'b"), "{v}");
    }
}

/// An item of a constant array or tuple: a `bool` item is a one-bit
/// literal, and an integer item beside it a number.
#[test]
fn an_item_of_a_constant_array_is_typed_too() {
    let (h, v) = both(PickArr::<Arr>::lowered);
    assert!(h.contains("x <= a when (not true) else b;"), "{h}");
    assert!(h.contains("y <= a when (not true) else b;"), "{h}");
    assert!(h.contains("z <= '1' when (a = to_unsigned(3, 8)) else '0';"));
    assert!(v.contains("assign x = ((~1'b1) ? a : b);"), "{v}");
    assert!(v.contains("assign y = ((~1'b1) ? a : b);"), "{v}");
    assert!(v.contains("assign z = (a == 8'h3);"), "{v}");
}

/// An `if` statement on the constant folds to the branch it takes.
#[test]
fn an_if_statement_on_the_constant_folds() {
    for (l, kept, gone) in [
        (PickReg::<Yes>::lowered("p"), "r <= a;", "r <= b;"),
        (PickReg::<No>::lowered("p"), "r <= b;", "r <= a;"),
    ] {
        let (h, v) = (l.vhdl(), l.verilog());
        assert!(h.contains(kept) && !h.contains(gone), "{h}");
        assert!(v.contains(kept) && !v.contains(gone), "{v}");
        assert!(!v.contains("if (1") && !v.contains("if ((~"), "{v}");
    }
}

/// A register's reset value taken from the constant.
#[test]
fn a_reset_value_from_the_constant_is_its_bit() {
    let (h, v) = both(PickRst::<Yes>::lowered);
    assert!(
        h.contains("r <= '1';") && v.contains("reg r = 1'h1;"),
        "{h}{v}"
    );
    let (h, v) = both(PickRst::<No>::lowered);
    assert!(
        h.contains("r <= '0';") && v.contains("reg r = 1'h0;"),
        "{h}{v}"
    );
}

/// Every expression of a lowered unit: its wires and the statements of
/// its processes, conditions included.
fn exprs(l: &Lowered) -> Vec<&Expr> {
    fn stmts<'a>(ss: &'a [Stmt], out: &mut Vec<&'a Expr>) {
        for s in ss {
            match s {
                Stmt::Drive(_, e) | Stmt::Guard(e) | Stmt::Check(_, e, _) => {
                    out.push(e)
                }
                Stmt::When(c, t, f) => {
                    out.push(c);
                    out.extend(t.iter().chain(f).map(|(_, e)| e));
                }
                Stmt::Case(arms) => {
                    for (c, d) in arms {
                        out.push(c);
                        out.extend(d.iter().map(|(_, e)| e));
                    }
                }
                Stmt::If(arms, els) => {
                    for (c, body) in arms {
                        out.push(c);
                        stmts(body, out);
                    }
                    stmts(els, out);
                }
            }
        }
    }
    let mut out: Vec<&Expr> = l.wires.iter().map(|(_, e)| e).collect();
    for p in &l.procs {
        stmts(&p.body, &mut out);
    }
    out
}

/// Whether a number stands anywhere in an expression.
fn has_num(e: &Expr) -> bool {
    match e {
        Expr::Num(_) => true,
        Expr::Name(_) | Expr::Bits(..) => false,
        Expr::Not(a)
        | Expr::Slice(a, ..)
        | Expr::Sext(a, _)
        | Expr::Zext(a, _)
        | Expr::Cast(a, _) => has_num(a),
        Expr::Bin(_, a, b) | Expr::Cat(a, b) | Expr::Index(a, b) => {
            has_num(a) || has_num(b)
        }
        Expr::Cond(c, a, b) => has_num(c) || has_num(a) || has_num(b),
    }
}

/// Every unit of this file that reads a `bool` constant, for both of
/// its values, and none other: the units with a bare number are not
/// here.
fn truth_units() -> Vec<(&'static str, Lowered)> {
    vec![
        ("Pick<Yes>", Pick::<Yes>::lowered("p")),
        ("Pick<No>", Pick::<No>::lowered("p")),
        ("PickIf<Yes>", PickIf::<Yes>::lowered("p")),
        ("PickIf<No>", PickIf::<No>::lowered("p")),
        ("PickAnd<Yes>", PickAnd::<Yes>::lowered("p")),
        ("PickAnd<No>", PickAnd::<No>::lowered("p")),
        ("PickBare<Yes>", PickBare::<Yes>::lowered("p")),
        ("PickBare<No>", PickBare::<No>::lowered("p")),
        ("PickMatch<Yes>", PickMatch::<Yes>::lowered("p")),
        ("PickMatch<No>", PickMatch::<No>::lowered("p")),
        ("PickEq<Yes>", PickEq::<Yes>::lowered("p")),
        ("PickEq<No>", PickEq::<No>::lowered("p")),
        ("CheckConst<Yes>", CheckConst::<Yes>::lowered("p")),
        ("CheckConst<No>", CheckConst::<No>::lowered("p")),
        ("PickNot<Yes>", PickNot::<Yes>::lowered("p")),
        ("PickNot<No>", PickNot::<No>::lowered("p")),
        ("PickNotIn<Yes>", PickNotIn::<Yes>::lowered("p")),
        ("PickNotIn<No>", PickNotIn::<No>::lowered("p")),
        ("PickNotBare<Yes>", PickNotBare::<Yes>::lowered("p")),
        ("PickNotBare<No>", PickNotBare::<No>::lowered("p")),
        ("PickIfNot<Yes>", PickIfNot::<Yes>::lowered("p")),
        ("PickIfNot<No>", PickIfNot::<No>::lowered("p")),
        ("PickNotOr<Yes>", PickNotOr::<Yes>::lowered("p")),
        ("PickNotOr<No>", PickNotOr::<No>::lowered("p")),
        ("PickNotReg<Yes>", PickNotReg::<Yes>::lowered("p")),
        ("PickNotReg<No>", PickNotReg::<No>::lowered("p")),
        ("PickNotEq<Yes>", PickNotEq::<Yes>::lowered("p")),
        ("PickNotEq<No>", PickNotEq::<No>::lowered("p")),
        ("PickNotOrBool<Yes>", PickNotOrBool::<Yes>::lowered("p")),
        ("PickNotOrBool<No>", PickNotOrBool::<No>::lowered("p")),
        ("PickNested<Yes>", PickNested::<Yes>::lowered("p")),
        ("PickNested<No>", PickNested::<No>::lowered("p")),
        ("PickLetAnd<Yes>", PickLetAnd::<Yes>::lowered("p")),
        ("PickLetAnd<No>", PickLetAnd::<No>::lowered("p")),
        ("PickMatchNot<Yes>", PickMatchNot::<Yes>::lowered("p")),
        ("PickMatchNot<No>", PickMatchNot::<No>::lowered("p")),
        ("CheckNot<Yes>", CheckNot::<Yes>::lowered("p")),
        ("CheckNot<No>", CheckNot::<No>::lowered("p")),
        ("PickGen<true>", PickGen::<true>::lowered("p")),
        ("PickGen<false>", PickGen::<false>::lowered("p")),
        ("PickReg<Yes>", PickReg::<Yes>::lowered("p")),
        ("PickReg<No>", PickReg::<No>::lowered("p")),
        ("PickNe<Yes>", PickNe::<Yes>::lowered("p")),
        ("PickNe<No>", PickNe::<No>::lowered("p")),
        ("PickAlias<Yes>", PickAlias::<Yes>::lowered("p")),
        ("PickAlias<No>", PickAlias::<No>::lowered("p")),
        ("PickRst<Yes>", PickRst::<Yes>::lowered("p")),
        ("PickRst<No>", PickRst::<No>::lowered("p")),
    ]
}

/// A `bool` constant lowers as a one-bit literal, by its type, so no
/// number stands anywhere in a unit that reads only `bool` constants
/// and signals: not under `!`, `==`, `&&`, `||` or `|`, not in a
/// branch of a choice, not behind a `let`, not in a check, a `match`
/// or a statement `if` (issue 2083).
#[test]
fn no_number_stands_where_a_truth_value_is() {
    for (name, l) in truth_units() {
        for e in exprs(&l) {
            assert!(!has_num(e), "{name}: {e:?}");
        }
    }
}

/// The same units in text: the VHDL is legal, and the Verilog holds no
/// unsized `0` or `1` under an operator, a comparison or a choice.
#[test]
fn every_form_of_the_constant_is_one_bit_in_the_text() {
    for (name, l) in truth_units() {
        let (h, v) = (l.vhdl(), l.verilog());
        legal(&h);
        for bad in [
            "~1)", "~0)", "(1 ", "(0 ", " 1)", " 0)", " 1 ", " 0 ", "!1", "!0",
        ] {
            assert!(!v.contains(bad), "{name}: {bad} in {v}");
        }
    }
    // Each form chooses as the run does, in Verilog, for both values.
    let line = |l: Lowered| l.verilog();
    for (v, want) in [
        (
            line(PickNotEq::<Yes>::lowered("p")),
            "assign out_rw = (((~1'b1) == 1'b0) ? a : b);",
        ),
        (
            line(PickNotEq::<No>::lowered("p")),
            "assign out_rw = (((~1'b0) == 1'b0) ? a : b);",
        ),
        (
            line(PickNotOrBool::<Yes>::lowered("p")),
            "assign out_rw = (((~1'b1) | x) ? a : b);",
        ),
        (
            line(PickGen::<true>::lowered("p")),
            "assign out_rw = ((~1'b1) ? a : b);",
        ),
        (
            line(PickGen::<false>::lowered("p")),
            "assign out_rw = ((~1'b0) ? a : b);",
        ),
    ] {
        assert!(v.contains(want), "{want} in {v}");
    }
    let v = line(PickNested::<Yes>::lowered("p"));
    assert!(v.contains("(s ? (~1'b1) : 1'b0)"), "{v}");
    let v = line(PickLetAnd::<Yes>::lowered("p"));
    assert!(v.contains("((~1'b1) && (a == b))"), "{v}");
    let v = line(CheckNot::<Yes>::lowered("p"));
    assert!(v.contains("assert (((~1'b1) || (a == b)));"), "{v}");
}

/// A number, not a truth value: a `u8` constant of one.
pub trait MaskConfig: Default {
    const ONE: u8;
}

#[derive(Default)]
pub struct One;
impl MaskConfig for One {
    const ONE: u8 = 1;
}

/// The negation of a number, in a value and inside a comparison that
/// selects: bitwise at the value's width, never sized to one bit.
#[derive(Default, Trace)]
pub struct MaskNot<C: MaskConfig> {
    _c: PhantomData<C>,
}

#[lower]
impl<C: MaskConfig> Unit for MaskNot<C> {
    async fn run(
        &mut self,
        (x, a, b): (In<U<8>>, In<U<8>>, In<U<8>>),
        (m, s): (Out<U<8>>, Out<U<8>>),
    ) {
        loop {
            DefaultClock::rising().await;
            m.set(x.get() & !U::from(C::ONE));
            s.set(mux(
                (x.get() & !U::from(C::ONE)) == a.get(),
                a.get(),
                b.get(),
            ));
        }
    }
}

/// A number's negation is left alone, in a value and in a comparison
/// that selects: bitwise at the value's width, `x & 0xFE`, and never a
/// one-bit literal. Verilog sizes `~1` there by its context, eight bits
/// against `x` and `a`, which is the run's value.
#[test]
fn the_negation_of_a_number_is_not_one_bit() {
    let (h, v) = both(MaskNot::<One>::lowered);
    assert!(h.contains("m <= (x and (not to_unsigned(1, 8)));"), "{h}");
    assert!(
        h.contains("s <= a when ((x and (not to_unsigned(1, 8))) = a) else b;"),
        "{h}"
    );
    assert!(v.contains("assign m = (x & (~1));"), "{v}");
    assert!(v.contains("assign s = (((x & (~1)) == a) ? a : b);"), "{v}");
    assert!(!v.contains("1'b"), "{v}");
}
