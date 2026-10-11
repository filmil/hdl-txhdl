// SPDX-License-Identifier: Apache-2.0
//! A port group (issue 1647): a struct of plain values marked
//! `#[value(ports)]` is one wire of the whole, and `split` gives an end
//! per field. A field's reading end reads that field of the wire as it
//! stands, and a field's driving end drives that field and leaves the
//! others, at once, as a wire is driven.
use txhdl::comp::{signal, DefaultClock, Group};
use txhdl::types::{Bit, Value, U};
use txhdl::Value;

#[derive(Value, Clone, Copy, Default, Debug, PartialEq)]
#[value(ports)]
pub struct Pins {
    pub rst: Bit,
    pub irq: Bit,
    pub data: U<4>,
}

#[derive(Value, Clone, Copy, Default, Debug, PartialEq)]
pub struct Packed {
    pub a: Bit,
    pub b: U<3>,
}

/// Whether `V` is a port group, as a lowering asks it.
fn is_group<V: Value>() -> bool {
    V::PORTS
}

#[test]
fn a_group_says_it_is_one_and_a_plain_struct_does_not() {
    assert!(is_group::<Pins>());
    assert!(!is_group::<Packed>());
    assert_eq!(<Pins as Value>::WIDTH, 6);
}

#[test]
fn a_fields_reading_end_reads_that_field_as_it_stands() {
    let (o, i) = signal::<Pins, DefaultClock>();
    let ends = i.split();
    o.set(Pins {
        rst: Bit::One,
        irq: Bit::Zero,
        data: U::<4>::from(9u8),
    });
    assert_eq!(ends.rst.get(), Bit::One);
    assert_eq!(ends.irq.get(), Bit::Zero);
    assert_eq!(ends.data.get(), U::<4>::from(9u8));
    o.set(Pins {
        rst: Bit::Zero,
        irq: Bit::One,
        data: U::<4>::from(3u8),
    });
    assert_eq!(ends.rst.get(), Bit::Zero, "read again, not kept");
    assert_eq!(ends.irq.get(), Bit::One);
}

#[test]
fn a_fields_driving_end_drives_that_field_and_leaves_the_others() {
    let (o, i) = signal::<Pins, DefaultClock>();
    let ends = o.split();
    ends.data.set(U::<4>::from(12u8));
    ends.irq.set(Bit::One);
    assert_eq!(
        i.get(),
        Pins {
            rst: Bit::Zero,
            irq: Bit::One,
            data: U::<4>::from(12u8),
        }
    );
    ends.irq.set(Bit::Zero);
    assert_eq!(i.get().data, U::<4>::from(12u8), "data kept");
    assert_eq!(i.get().irq, Bit::Zero);
}

#[test]
fn a_group_splits_through_the_trait_too() {
    let (o, i) = signal::<Pins, DefaultClock>();
    let ins = <Pins as Group>::split_in(i);
    o.set(Pins {
        rst: Bit::One,
        ..Pins::default()
    });
    assert_eq!(ins.rst.get(), Bit::One);
}
