// SPDX-License-Identifier: Apache-2.0
//! A port group (issue 1647): a struct of plain values marked
//! `#[value(ports)]` is one wire of the whole, and `split` gives an end
//! per field. A field's reading end reads that field of the wire as it
//! stands, and a field's driving end drives that field and leaves the
//! others, at once, as a wire is driven.
use txhdl::comp::{signal, Clock, DefaultClock, Group, In, Out, Unit};
use txhdl::types::{Bit, Value, U};
use txhdl::{lower, Trace, Value};

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

/// Two lines under their own names, as a board's pins are.
#[derive(Value, Clone, Copy, Default, Debug, PartialEq)]
#[value(bare, ports)]
pub struct Lines {
    pub a: Bit,
    pub b: Bit,
}

/// A unit taking a bare group and a prefixed one, and driving a bit.
#[derive(Trace, Default)]
pub struct Reads {}

#[lower]
impl Unit for Reads {
    async fn run(&mut self, (l, p): (In<Lines>, In<Pins>), o: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            o.set(l.get().a & p.get().rst);
        }
    }
}

/// Two bare groups of one type, whose fields would be the same ports.
#[derive(Trace, Default)]
pub struct Twice {}

#[lower]
impl Unit for Twice {
    async fn run(&mut self, (x, y): (In<Lines>, In<Lines>), o: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            o.set(x.get().a & y.get().b);
        }
    }
}

#[test]
fn a_bare_group_is_its_fields_and_a_group_is_prefixed() {
    // The words of `#[value(...)]` are read in any order.
    assert!(is_group::<Lines>());
    let v = Reads::lowered("reads").verilog();
    for port in ["input a", "input b", "input p_rst", "input p_irq"] {
        assert!(v.contains(port), "no `{port}` in:\n{v}");
    }
    assert!(v.contains("wire [1:0] l = {a, b};"), "{v}");
}

#[test]
#[should_panic(expected = "is a port of `twice` twice")]
fn two_bare_groups_with_one_field_name_are_refused() {
    let _ = Twice::lowered("twice");
}
