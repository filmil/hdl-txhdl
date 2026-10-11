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
        (ss, sz, small): (Out<U<64>>, Out<U<64>>, Out<U<16>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let x = a.get().sext::<64>();
            ss.set(x.mul::<64>(b.get().sext::<64>()));
            sz.set(a.get().sext::<64>().mul::<64>(h.get().resize::<64>()));
            small.set(c.get().sext::<16>().mul::<16>(d.get().resize::<16>()));
        }
    }
}

/// Two signed words, through a `let` or not, and a signed word by an
/// unsigned half: each a signed product of the narrow values, in the
/// Verilog and in the VHDL. A signed byte by an unsigned byte into
/// sixteen bits is too narrow for that, and stays as it was.
#[test]
fn a_product_of_extended_operands_is_signed_at_their_widths() {
    let l = Products::lowered("p");
    let v = l.verilog();
    let h = l.vhdl();
    for want in [
        "wire [63:0] sl0 = $signed(a) * $signed(b);",
        "wire [63:0] sl1 = $signed(a) * $signed({1'b0, h});",
        "({{8{c[7]}}, c} * {{8{1'b0}}, d})",
    ] {
        assert!(v.contains(want), "no `{want}` in:\n{v}");
    }
    for want in [
        "sl0 <= unsigned(resize(signed(a) * signed(b), 64));",
        "sl1 <= unsigned(resize(signed(a) * signed('0' & h), 64));",
    ] {
        assert!(h.contains(want), "no `{want}` in:\n{h}");
    }
    assert_eq!(v.matches("$signed").count(), 4, "{v}");
}
