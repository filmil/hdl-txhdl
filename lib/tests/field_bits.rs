// SPDX-License-Identifier: Apache-2.0
//! The bits a chain of fields names, as `#[derive(Value)]` finds them
//! through each field's type, and the slice a lowering takes of them
//! (issue 1648).
use txhdl::netlist::{field_path, Expr};
use txhdl::types::{Bit, Value, U};
use txhdl::Value;

#[derive(Value, Clone, Copy, Default, Debug, PartialEq)]
pub struct Flags {
    pub err: Bit,
    pub code: U<3>,
}

#[derive(Value, Clone, Copy, Default, Debug, PartialEq)]
pub struct Inner {
    pub hi: U<8>,
    pub flags: Flags,
}

/// Generic, so the derive's walk is written inside the struct's own
/// impl, where `U<N>` has a width.
#[derive(Value, Clone, Copy, Default, Debug, PartialEq)]
pub struct Outer<const N: usize> {
    pub a: U<N>,
    pub inner: Inner,
    pub last: Bit,
}

/// `Outer<5>` is `a` 5, `inner` 12 (`hi` 8, `flags` 4: `err` 1,
/// `code` 3), `last` 1: 18 bits, the first field highest.
#[test]
fn a_chain_of_fields_finds_its_bits_through_each_type() {
    type O = Outer<5>;
    assert_eq!(O::WIDTH, 18);
    assert_eq!(O::field_bits(&["a"]), Some((13, 5)));
    assert_eq!(O::field_bits(&["inner"]), Some((1, 12)));
    assert_eq!(O::field_bits(&["inner", "hi"]), Some((5, 8)));
    assert_eq!(O::field_bits(&["inner", "flags"]), Some((1, 4)));
    assert_eq!(O::field_bits(&["inner", "flags", "err"]), Some((4, 1)));
    assert_eq!(O::field_bits(&["inner", "flags", "code"]), Some((1, 3)));
    assert_eq!(O::field_bits(&["last"]), Some((0, 1)));
}

/// A field the value does not have, at any depth, a field of a scalar,
/// and no field at all, are none.
#[test]
fn a_chain_that_names_no_field_is_none() {
    type O = Outer<5>;
    assert_eq!(O::field_bits(&["nope"]), None);
    assert_eq!(O::field_bits(&["inner", "nope"]), None);
    assert_eq!(O::field_bits(&["last", "x"]), None);
    assert_eq!(O::field_bits(&["a", "x"]), None);
    assert_eq!(O::field_bits(&[]), None);
}

/// One bit is an index of the value, wider fields a slice of it.
#[test]
fn a_path_to_one_bit_is_an_index_and_a_wider_one_a_slice() {
    let p = || Expr::Name("p".into());
    let bit = format!(
        "{:?}",
        field_path::<Outer<5>>(p(), &["inner", "flags", "err"])
    );
    assert!(bit.starts_with("Index(Name(\"p\"), Num(4))"), "{bit}");
    let code = format!(
        "{:?}",
        field_path::<Outer<5>>(p(), &["inner", "flags", "code"])
    );
    assert_eq!(code, "Slice(Name(\"p\"), 1, 3)");
}

/// A chain that names no field is a mistake the run has already
/// refused, so it is said rather than lowered.
#[test]
#[should_panic(expected = "`inner.nope` is not a field of the value")]
fn a_path_to_no_field_is_refused() {
    field_path::<Outer<5>>(Expr::Name("p".into()), &["inner", "nope"]);
}
