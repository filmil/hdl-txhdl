// SPDX-License-Identifier: Apache-2.0
//! Arrays of ports inside a struct of ports (issue 1636): a struct of
//! ports may hold `[In<Bit>; N]`, its ports `side_field_0` onward, read
//! by index in `run` as an array side is.
use txhdl::comp::{Clock, DefaultClock, In, Out, Unit};
use txhdl::types::Bit;
use txhdl::{lower, Ports, Trace};

/// Two flags, as one field of a struct of ports.
#[derive(Ports)]
pub struct Flags {
    pub f: [In<Bit>; 2],
}

/// A side holding the flags and an enable.
#[derive(Ports)]
pub struct Inp {
    pub flags: Flags,
    pub en: In<Bit>,
}

/// Whether any flag is up, while enabled.
#[derive(Trace, Default)]
pub struct Any {}

#[lower]
impl Unit for Any {
    async fn run(&mut self, inp: Inp, any: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            any.set(
                inp.en.get() & (inp.flags.f[0].get() | inp.flags.f[1].get()),
            );
        }
    }
}

#[test]
fn an_array_in_a_struct_of_ports_is_a_port_per_element() {
    let v = Any::lowered("any").verilog();
    for port in ["input inp_flags_f_0", "input inp_flags_f_1", "input inp_en"] {
        assert!(v.contains(port), "no `{port}` in:\n{v}");
    }
    assert!(v.contains("inp_flags_f_0 | inp_flags_f_1"), "{v}");
}

/// The same, with the flags read in a loop, as a unit of lanes reads its
/// lanes.
#[derive(Trace, Default)]
pub struct AnyLoop {}

#[lower]
impl Unit for AnyLoop {
    async fn run(&mut self, inp: Inp, any: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            let mut up = Bit::Zero;
            for i in 0..2 {
                up = up | inp.flags.f[i].get();
            }
            any.set(inp.en.get() & up);
        }
    }
}

#[test]
fn an_array_in_a_struct_of_ports_is_read_in_a_loop() {
    let v = AnyLoop::lowered("any_loop").verilog();
    for port in ["input inp_flags_f_0", "input inp_flags_f_1"] {
        assert!(v.contains(port), "no `{port}` in:\n{v}");
    }
    // The loop unrolled, each lane folded into the last.
    assert!(v.contains("| inp_flags_f_0)"), "{v}");
    assert!(v.contains("| inp_flags_f_1)"), "{v}");
}

/// A tuple side holding a struct of ports beside an array of ports.
#[derive(Trace, Default)]
pub struct Beside {}

#[lower]
impl Unit for Beside {
    async fn run(&mut self, (inp, more): (Inp, [In<Bit>; 2]), any: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            any.set(inp.flags.f[0].get() & more[1].get() & inp.en.get());
        }
    }
}

#[test]
fn a_tuple_side_holds_a_struct_beside_an_array() {
    let v = Beside::lowered("beside").verilog();
    for port in ["input inp_flags_f_0", "input inp_en", "input more_1"] {
        assert!(v.contains(port), "no `{port}` in:\n{v}");
    }
}

/// Two lines as a port group.
#[derive(txhdl::Value, Clone, Copy, Default, Debug)]
#[value(ports)]
pub struct Pins {
    pub rst: Bit,
    pub irq: Bit,
}

/// Either line of an array, while enabled.
#[derive(Trace, Default)]
pub struct Or2 {}

#[lower]
impl Unit for Or2 {
    async fn run(&mut self, (ins, en): ([In<Bit>; 2], In<Bit>), o: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            o.set((ins[0].get() | ins[1].get()) & en.get());
        }
    }
}

/// A unit of units handing a child an array made of a split group's
/// fields.
#[derive(Trace, Default)]
pub struct Hands {
    pub c: Or2,
}

#[lower]
impl Unit for Hands {
    async fn run(&mut self, (k, en): (In<Pins>, In<Bit>), o: Out<Bit>) {
        let k = k.split();
        self.c.run(([k.rst, k.irq], en), o).await;
    }
}

#[test]
fn an_array_passed_to_a_child_holds_field_paths() {
    let v = Hands::lowered("hands").verilog();
    for join in [".ins_0(k_rst)", ".ins_1(k_irq)", ".en(en)"] {
        assert!(v.contains(join), "no `{join}` in:\n{v}");
    }
}

/// One lane of a bus, as a struct of ports.
#[derive(Ports)]
pub struct Lane {
    pub valid: In<Bit>,
}

/// A bus of lanes, each a struct.
#[derive(Ports)]
pub struct Lanes {
    pub lanes: [Lane; 2],
}

/// Whether the second lane is valid, through a path after the index.
#[derive(Trace, Default)]
pub struct Second {}

#[lower]
impl Unit for Second {
    async fn run(&mut self, bus: Lanes, o: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            o.set(bus.lanes[1].valid.get());
        }
    }
}

#[test]
fn a_path_goes_on_after_an_index() {
    let v = Second::lowered("second").verilog();
    for port in ["input bus_lanes_0_valid", "input bus_lanes_1_valid"] {
        assert!(v.contains(port), "no `{port}` in:\n{v}");
    }
    assert!(v.contains("assign o = bus_lanes_1_valid;"), "{v}");
}

/// A lane that holds an array of its own.
#[derive(Ports)]
pub struct Wide {
    pub sub: [In<Bit>; 2],
}

/// A side that is an array of structs of ports, read through the path
/// after the index, by a constant and in a loop, and an index nested in
/// the path.
#[derive(Trace, Default)]
pub struct Direct {}

// The lowering unrolls a loop over ports by its index, which Clippy would
// have an iterator.
#[allow(clippy::needless_range_loop)]
#[lower]
impl Unit for Direct {
    async fn run(
        &mut self,
        (lanes, wide): ([Lane; 2], [Wide; 2]),
        (first, all, deep): (Out<Bit>, Out<Bit>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            first.set(lanes[1].valid.get());
            let mut every = Bit::One;
            for i in 0..2 {
                every = every & lanes[i].valid.get();
            }
            all.set(every);
            deep.set(wide[0].sub[1].get());
        }
    }
}

#[test]
fn a_side_of_structs_is_read_through_its_paths() {
    let v = Direct::lowered("direct").verilog();
    for port in [
        "input lanes_0_valid",
        "input lanes_1_valid",
        "input wide_0_sub_1",
        "input wide_1_sub_0",
    ] {
        assert!(v.contains(port), "no `{port}` in:\n{v}");
    }
    assert!(v.contains("assign first = lanes_1_valid;"), "{v}");
    assert!(v.contains("& lanes_0_valid)"), "{v}");
    assert!(v.contains("& lanes_1_valid)"), "{v}");
    assert!(v.contains("assign deep = wide_0_sub_1;"), "{v}");
}

/// A struct of ports named as a channel's end is, but taking no type.
pub mod uart {
    use txhdl::comp::In;
    use txhdl::types::Bit;
    use txhdl::Ports;

    /// The line a transmitter drives.
    #[derive(Ports)]
    pub struct Tx {
        pub d: In<Bit>,
    }
}

/// A tuple side holding a port written by its path and a struct of
/// ports called `Tx`.
#[derive(Trace, Default)]
pub struct Named {}

#[lower]
impl Unit for Named {
    async fn run(
        &mut self,
        (q, tx): (txhdl::comp::In<Bit>, uart::Tx),
        o: Out<Bit>,
    ) {
        loop {
            DefaultClock::rising().await;
            o.set(q.get() & tx.d.get());
        }
    }
}

#[test]
fn an_end_by_its_path_beside_a_struct_called_tx() {
    let v = Named::lowered("named").verilog();
    for port in ["input q", "input tx_d"] {
        assert!(v.contains(port), "no `{port}` in:\n{v}");
    }
    assert!(v.contains("assign o = (q & tx_d);"), "{v}");
}
