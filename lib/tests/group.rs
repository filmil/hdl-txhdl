// SPDX-License-Identifier: Apache-2.0
//! A port group (issue 1647): a struct of plain values marked
//! `#[value(ports)]` is one wire of the whole, and `split` gives an end
//! per field. A field's reading end reads that field of the wire as it
//! stands, and a field's driving end drives that field and leaves the
//! others, at once, as a wire is driven.
use txhdl::comp::{join2, signal, Clock, DefaultClock, Group, In, Out, Unit};
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
    // `p_rst` is a field of a group named for its port, not the reset,
    // which the module still has once (issue 1715).
    let top = v.split("endmodule").next().unwrap();
    assert_eq!(top.matches("input rst,").count(), 1, "{top}");
}

#[test]
#[should_panic(expected = "is a port of `twice` twice")]
fn two_bare_groups_with_one_field_name_are_refused() {
    let _ = Twice::lowered("twice");
}

/// The board-clock lines of a unit of units, bare, its reset among them
/// (issue 1715).
#[derive(Value, Clone, Copy, Default, Debug, PartialEq)]
#[value(ports, bare)]
pub struct SysP {
    pub rst: Bit,
    pub irq: Bit,
}

/// Lines on another clock, named for their port.
#[derive(Value, Clone, Copy, Default, Debug, PartialEq)]
#[value(ports)]
pub struct ScanP {
    pub sel: Bit,
    pub tdi: Bit,
}

/// Two lamps, each driven by its own child.
#[derive(Value, Clone, Copy, Default, Debug, PartialEq)]
#[value(ports, bare)]
pub struct LedsP {
    pub halt: Bit,
    pub lamp: Bit,
}

/// A side taken apart, its fields port groups.
pub struct PIn {
    pub sys: In<SysP>,
    pub scan: In<ScanP>,
}

/// The other side, one group.
pub struct POut {
    pub leds: Out<LedsP>,
}

/// Both of two lines.
#[derive(Trace, Default)]
pub struct Wire1 {}

#[lower]
impl Unit for Wire1 {
    async fn run(&mut self, (a, b): (In<Bit>, In<Bit>), o: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            o.set(a.get() & b.get());
        }
    }
}

/// Two children, each given a line of each input group and a lamp.
#[derive(Trace, Default)]
pub struct PairTop {
    pub one: Wire1,
    pub two: Wire1,
}

#[lower]
impl Unit for PairTop {
    async fn run(&mut self, PIn { sys, scan }: PIn, POut { leds }: POut) {
        let s = sys.split();
        let c = scan.split();
        let l = leds.split();
        join2(
            self.one.run((s.rst, c.sel), l.halt),
            self.two.run((s.irq, c.tdi), l.lamp),
        )
        .await;
    }
}

#[test]
fn groups_in_a_side_taken_apart_and_a_reset_among_them() {
    let v = PairTop::lowered("pair").verilog();
    let at = v.find("module pair(").expect("the parent's module");
    let top = v[at..].split("endmodule").next().unwrap();
    assert_eq!(top.matches("input rst,").count(), 1, "one reset:\n{top}");
    let h = PairTop::vhdl("pair");
    let at = h.find("entity pair is").expect("the parent's entity");
    let entity = h[at..].split("end entity").next().unwrap();
    let resets = entity.matches("rst : in std_logic").count();
    assert_eq!(resets, 1, "one reset:\n{entity}");
    for port in [
        "input rst",
        "input irq",
        "input scan_sel",
        "input scan_tdi",
        "output halt",
        "output lamp",
    ] {
        assert!(v.contains(port), "no `{port}` in:\n{v}");
    }
}

/// A reset a unit asks for, as a group named for its port.
#[derive(Value, Clone, Copy, Default, Debug, PartialEq)]
#[value(ports)]
pub struct Ask {
    pub rst: Bit,
}

/// The same, bare, so that its field would be `rst` at the edge.
#[derive(Value, Clone, Copy, Default, Debug, PartialEq)]
#[value(ports, bare)]
pub struct AskBare {
    pub rst: Bit,
}

/// Passes a line on as a reset request.
#[derive(Trace, Default)]
pub struct Asks {}

#[lower]
impl Unit for Asks {
    async fn run(&mut self, i: In<Bit>, o: Out<Ask>) {
        loop {
            DefaultClock::rising().await;
            o.set(Ask { rst: i.get() });
        }
    }
}

/// The same, with the bare group.
#[derive(Trace, Default)]
pub struct AsksBare {}

#[lower]
impl Unit for AsksBare {
    async fn run(&mut self, i: In<Bit>, o: Out<AskBare>) {
        loop {
            DefaultClock::rising().await;
            o.set(AskBare { rst: i.get() });
        }
    }
}

#[test]
fn an_output_group_named_for_its_port_may_hold_rst() {
    let v = Asks::lowered("asks").verilog();
    assert!(v.contains("output o_rst"), "{v}");
    assert_eq!(v.matches("input rst,").count(), 1, "{v}");
}

#[test]
#[should_panic(expected = "that is not one bit in")]
fn a_bare_output_group_holding_rst_is_refused() {
    let _ = AsksBare::lowered("asks_bare").verilog();
}
