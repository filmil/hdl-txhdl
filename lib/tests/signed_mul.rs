// SPDX-License-Identifier: Apache-2.0
//! A product of extended operands is written at the operands' own
//! widths, signed where an operand was sign-extended, so that Vivado
//! maps it to the DSP blocks its real width needs (issue 1649).
use txhdl::comp::{Clock, DefaultClock, In, Out, Unit};
use txhdl::types::U;
use txhdl::{lower, Trace};

#[derive(Trace, Default)]
pub struct Products {}

#[lower]
impl Unit for Products {
    async fn run(
        &mut self,
        (a, b, h, c, d): (In<U<32>>, In<U<32>>, In<U<16>>, In<U<8>>, In<U<8>>),
        (ss, sz, small, inline, cut): (
            Out<U<64>>,
            Out<U<64>>,
            Out<U<16>>,
            Out<U<64>>,
            Out<U<12>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let x = a.get().sext::<64>();
            ss.set(x.mul::<64>(b.get().sext::<64>()));
            sz.set(a.get().sext::<64>().mul::<64>(h.get().resize::<64>()));
            small.set(c.get().sext::<16>().mul::<16>(d.get().resize::<16>()));
            inline.set(a.get().sext::<64>().mul::<64>(b.get().sext::<64>()));
            cut.set(c.get().sext::<12>().mul::<12>(d.get().resize::<12>()));
        }
    }
}

/// Two signed words, through a `let` and inline, a signed word by an
/// unsigned half, and a signed byte by an unsigned byte into sixteen
/// bits: each a signed product of the narrow values, in the Verilog and
/// in the VHDL. Into twelve bits, narrower than the two bytes together,
/// the product would be cut, and stays as it was.
#[test]
fn a_product_of_extended_operands_is_signed_at_their_widths() {
    let l = Products::lowered("p");
    let v = l.verilog();
    let h = l.vhdl();
    for want in [
        "= $signed(a) * $signed(b);",
        "= $signed(a) * $signed({1'b0, h});",
        "= $signed(c) * $signed({1'b0, d});",
        "({{4{c[7]}}, c} * {{4{1'b0}}, d})",
    ] {
        assert!(v.contains(want), "no `{want}` in:\n{v}");
    }
    for want in [
        "unsigned(resize(signed(a) * signed(b), 64));",
        "unsigned(resize(signed(a) * signed('0' & (h)), 64));",
        "unsigned(resize(signed(c) * signed('0' & (d)), 16));",
    ] {
        assert!(h.contains(want), "no `{want}` in:\n{h}");
    }
    // Four wires: the two products of signed words are written apart,
    // one read through the `let` and one inline, since the pass merges
    // only an expression written the same way twice.
    assert_eq!(v.matches("$signed").count(), 8, "{v}");
}

/// The VHDL alone, with no Verilog written before it on the thread,
/// finds the same products: each emitter runs the pass that marks them.
#[test]
fn the_vhdl_alone_writes_the_products_signed() {
    let h = Products::lowered("p").vhdl();
    assert!(
        h.contains("unsigned(resize(signed(a) * signed(b), 64));"),
        "{h}"
    );
}
