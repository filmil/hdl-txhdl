// SPDX-License-Identifier: Apache-2.0
//! A field whose type is written as a path, `m::X` or
//! `txhdl::types::U<3>`, derives as one written by its last name
//! (issue 1700).
use txhdl::netlist::Fields;
use txhdl::types::Value;
use txhdl::{Trace, Value};

pub mod m {
    #[derive(Clone, Default)]
    pub struct X;
    impl txhdl::comp::trace::Traceable for X {
        fn trace(&self, _: &txhdl::comp::trace::Scope) {}
    }
    impl txhdl::netlist::Port for X {}
}

#[derive(Trace, Default)]
pub struct T {
    pub x: m::X,
    pub r: txhdl::comp::Reg<txhdl::types::U<8>>,
}

#[derive(Value, Clone, Copy, Default, Debug, PartialEq)]
pub struct P {
    pub a: txhdl::types::Bit,
    pub b: txhdl::types::U<3>,
}

/// The unit's fields are both there, the register eight bits wide.
#[test]
fn a_trace_field_may_be_typed_by_a_path() {
    let f = T::fields();
    let names: Vec<_> = f.iter().map(|e| e.0).collect();
    assert_eq!(names, ["x", "r"]);
    assert_eq!(f[1].2, 8);
}

/// The value is as wide as its two fields together.
#[test]
fn a_value_field_may_be_typed_by_a_path() {
    assert_eq!(<P as Value>::WIDTH, 4);
}
