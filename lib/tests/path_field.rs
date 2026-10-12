// SPDX-License-Identifier: Apache-2.0
//! A field whose type is written as a path, `m::X`, `::txhdl::types::Bit`
//! or `txhdl::types::U<3>`, derives as the same type written by a name
//! in scope would (issue 1700).
use txhdl::comp::trace::Kind;
use txhdl::netlist::{Fields, Ports};
use txhdl::types::Value;
use txhdl::{Ports, Trace, Value};

pub mod m {
    #[derive(Clone, Default)]
    pub struct X;
    impl txhdl::comp::trace::Traceable for X {
        fn trace(&self, _: &txhdl::comp::trace::Scope) {}
    }
    impl txhdl::netlist::Port for X {}

    pub mod deep {
        #[derive(txhdl::Value, Clone, Copy, Default, Debug, PartialEq)]
        pub struct V {
            pub lo: txhdl::types::U<2>,
            pub hi: txhdl::types::U<3>,
        }
    }
}

#[derive(Trace, Default)]
pub struct T {
    pub x: m::X,
    pub r: txhdl::comp::Reg<txhdl::types::U<8>>,
    pub s: ::txhdl::comp::Reg<::txhdl::types::U<4>>,
    #[rename("held")]
    q: ::txhdl::comp::Reg<::txhdl::types::Bit>,
}

#[derive(Value, Clone, Copy, Default, Debug, PartialEq)]
pub struct P {
    pub a: txhdl::types::Bit,
    pub b: txhdl::types::U<3>,
    pub c: ::txhdl::types::Bit,
    pub v: m::deep::V,
}

#[derive(Ports)]
pub struct Q {
    pub go: txhdl::comp::In<txhdl::types::Bit>,
    pub data: ::txhdl::comp::Out<::txhdl::types::U<5>>,
}

/// Each field is there, by its netlist name: `x` an end of nothing,
/// `r`, `s` and `q` registers of their widths, `q` renamed `held`.
#[test]
fn a_trace_field_may_be_typed_by_a_path() {
    let f = T::fields();
    let got: Vec<_> = f.iter().map(|e| (e.0, e.1.is_some(), e.2)).collect();
    assert_eq!(
        got,
        [
            ("x", false, 0),
            ("r", true, 8),
            ("s", true, 4),
            ("held", true, 1)
        ]
    );
    // The attribute on a field typed from the root, and not `pub`, is
    // read for that field and no other (issue 1714).
    assert_eq!(T::RENAMES, [("q", "held")]);
}

/// The value is as wide as its fields together, a user's own value
/// written by a deep path among them, and its layout names each one.
#[test]
fn a_value_field_may_be_typed_by_a_path() {
    assert_eq!(<P as Value>::WIDTH, 1 + 3 + 1 + 5);
    assert_eq!(
        <P as Value>::layout(),
        [("a", 1), ("b", 3), ("c", 1), ("v", 5)]
    );
}

/// A struct of ports names each port, with its direction, whatever
/// path its end is written by.
#[test]
fn a_ports_field_may_be_typed_by_a_path() {
    let got: Vec<_> = <Q as Ports>::ports()
        .into_iter()
        .map(|p| (p.name, p.kind, p.width))
        .collect();
    assert_eq!(
        got,
        [
            ("go".to_string(), Kind::In, 1),
            ("data".to_string(), Kind::Out, 5)
        ]
    );
}
