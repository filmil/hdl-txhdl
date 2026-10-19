// SPDX-License-Identifier: Apache-2.0
//! `[T; N]` as a field of a `Value` struct (issue 1869): laid out
//! element 0 highest, as a struct's first field is, so `N` named fields
//! and an array of `N` have the same bits; read in a lowered body as
//! `v.lanes[k]`, `k` a number or a loop's variable.
use txhdl::comp::{
    chan, signal, Clock, DefaultClock, In, Out, Running, Rx, Unit,
};
use txhdl::types::{Bit, Value, U};
use txhdl::{lower, Trace, Transaction, Value};

/// A tag and four lanes of a byte.
#[derive(Value, Transaction, Clone, Copy, Default, Debug)]
pub struct Rec {
    pub tag: U<4>,
    pub lanes: [U<8>; 4],
}

/// The same bits, the lanes named one by one.
#[derive(Value, Clone, Copy, Default, Debug)]
pub struct Named {
    pub tag: U<4>,
    pub l0: U<8>,
    pub l1: U<8>,
    pub l2: U<8>,
    pub l3: U<8>,
}

/// The sum of a record's lanes, in a loop, and its third lane alone.
#[derive(Trace, Default)]
pub struct Sum {}

#[lower]
impl Unit<In<Rec>, (Out<U<10>>, Out<U<8>>)> for Sum {
    async fn run(&mut self, r: In<Rec>, (sum, third): (Out<U<10>>, Out<U<8>>)) {
        loop {
            DefaultClock::rising().await;
            let v = r.get();
            let mut s = U::<10>::from(0u16);
            for k in 0..4 {
                s = s + v.lanes[k].resize::<10>();
            }
            sum.set(s);
            third.set(v.lanes[2]);
        }
    }
}

/// A record taken off a channel, its lanes summed: the array travels
/// in a transaction.
#[derive(Trace, Default)]
pub struct Take {}

#[lower]
impl Unit<Rx<Rec>, Out<U<10>>> for Take {
    async fn run(&mut self, rx: Rx<Rec>, sum: Out<U<10>>) {
        loop {
            DefaultClock::rising().await;
            let v = rx.head();
            let _ = rx.recv_if(Bit::One);
            let mut s = U::<10>::from(0u16);
            for k in 0..4 {
                s = s + v.lanes[k].resize::<10>();
            }
            sum.set(s);
        }
    }
}

#[test]
fn an_array_is_laid_out_as_its_fields_would_be() {
    assert_eq!(<Rec as Value>::WIDTH, 36);
    assert_eq!(<Rec as Value>::WIDTH, <Named as Value>::WIDTH);
    for (k, n) in ["l0", "l1", "l2", "l3"].iter().enumerate() {
        let idx = k.to_string();
        assert_eq!(
            Rec::field_bits(&["lanes", &idx]),
            Named::field_bits(&[n]),
            "lane {k}"
        );
    }
    let r = Rec {
        tag: U::from(0xau8),
        lanes: [U::from(1u8), U::from(2u8), U::from(3u8), U::from(4u8)],
    };
    let n = Named {
        tag: U::from(0xau8),
        l0: U::from(1u8),
        l1: U::from(2u8),
        l2: U::from(3u8),
        l3: U::from(4u8),
    };
    assert_eq!(r.vcd(), n.vcd(), "the same bits");
}

#[test]
fn a_loop_reads_each_lane_in_the_simulation() {
    let (r_o, r_i) = signal::<Rec, DefaultClock>();
    let (s_o, s_i) = signal::<U<10>, DefaultClock>();
    let (t_o, t_i) = signal::<U<8>, DefaultClock>();
    let mut unit = Sum::default();
    let mut sim = Running::new(unit.run(r_i, (s_o, t_o)));
    r_o.set(Rec {
        tag: U::from(0u8),
        lanes: [U::from(200u8), U::from(100u8), U::from(7u8), U::from(255u8)],
    });
    sim.cycle();
    sim.cycle();
    assert_eq!(s_i.get().raw(), 562);
    assert_eq!(t_i.get().raw(), 7);
}

#[test]
fn each_lane_is_a_slice_of_the_port_in_the_netlist() {
    let v = Sum::lowered("sum").verilog();
    // Lane k of the 32 lane bits is bits 31 - 8k down to 24 - 8k, so
    // the loop adds them in that order, and lane 2 is bits 15 to 8.
    for k in 0..4 {
        let hi = 31 - 8 * k;
        let lo = 24 - 8 * k;
        let add = format!("{{{{2{{1'b0}}}}, r[{hi}:{lo}]}});");
        let at = v.find(&format!("assign s_l{k} =")).expect("the sum");
        let line = &v[at..at + v[at..].find('\n').unwrap()];
        assert!(line.ends_with(&add), "lane {k}: {line}");
    }
    assert!(v.contains("assign third = r[15:8];"), "{v}");
}

#[test]
fn an_array_travels_over_a_channel() {
    let (tx, rx) = chan::<Rec, DefaultClock>();
    let (s_o, s_i) = signal::<U<10>, DefaultClock>();
    let mut unit = Take::default();
    let mut sim = Running::new(unit.run(rx, s_o));
    tx.send(Rec {
        tag: U::from(1u8),
        lanes: [U::from(1u8), U::from(2u8), U::from(3u8), U::from(4u8)],
    });
    sim.cycle();
    sim.cycle();
    assert_eq!(s_i.get().raw(), 10);
    let v = Take::lowered("take").verilog();
    assert!(v.contains("rx_data[31:24]"), "{v}");
}

/// A pair of nibbles, as an array's element.
#[derive(Value, Clone, Copy, Default, Debug)]
pub struct Pair {
    pub hi: U<4>,
    pub lo: U<4>,
}

/// Two pairs, and a grid of two rows of two nibbles.
#[derive(Value, Clone, Copy, Default, Debug)]
pub struct Deep {
    pub pairs: [Pair; 2],
    pub grid: [[U<4>; 2]; 2],
}

/// The low nibble of each pair, the grid's corners, and indexes spelled
/// as Rust spells them.
#[derive(Trace, Default)]
pub struct Reach {}

#[lower]
impl Unit<In<Deep>, (Out<U<8>>, Out<U<8>>, Out<U<8>>)> for Reach {
    async fn run(
        &mut self,
        d: In<Deep>,
        (los, corners, spelled): (Out<U<8>>, Out<U<8>>, Out<U<8>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let v = d.get();
            let mut l = U::<8>::from(0u8);
            for k in 0..2 {
                l = (l << 4) | v.pairs[k].lo.resize::<8>();
            }
            los.set(l);
            corners.set(
                (v.grid[0][0].resize::<8>() << 4) | v.grid[1][1].resize::<8>(),
            );
            spelled.set(
                (v.grid[0x1][0usize].resize::<8>() << 4)
                    | v.pairs[1usize].hi.resize::<8>(),
            );
        }
    }
}

#[test]
fn an_element_of_structs_and_of_arrays_is_reached_by_path() {
    // Deep is 32 bits: pairs (16) then grid (16), each first-highest.
    assert_eq!(<Deep as Value>::WIDTH, 32);
    assert_eq!(Deep::field_bits(&["pairs", "0", "hi"]), Some((28, 4)));
    assert_eq!(Deep::field_bits(&["pairs", "1", "lo"]), Some((16, 4)));
    assert_eq!(Deep::field_bits(&["grid", "0", "0"]), Some((12, 4)));
    assert_eq!(Deep::field_bits(&["grid", "1", "1"]), Some((0, 4)));
    // Past the end, or not a number, is no element.
    assert_eq!(Deep::field_bits(&["pairs", "2"]), None);
    assert_eq!(Deep::field_bits(&["grid", "x"]), None);
}

#[test]
fn an_array_has_a_part_and_a_layout_per_element() {
    let r = Rec {
        tag: U::from(0u8),
        lanes: [U::from(1u8), U::from(2u8), U::from(3u8), U::from(4u8)],
    };
    let parts = r.lanes.parts();
    let names: Vec<&str> = parts.iter().map(|p| p.name).collect();
    assert_eq!(names, ["0", "1", "2", "3"]);
    assert_eq!(parts[2].bits, "00000011");
    assert_eq!(
        <[U<8>; 4] as Value>::layout(),
        vec![("0", 8), ("1", 8), ("2", 8), ("3", 8)]
    );
}

#[test]
fn nested_and_struct_elements_lower_to_their_slices() {
    let (d_o, d_i) = signal::<Deep, DefaultClock>();
    let (l_o, l_i) = signal::<U<8>, DefaultClock>();
    let (c_o, c_i) = signal::<U<8>, DefaultClock>();
    let (s_o, s_i) = signal::<U<8>, DefaultClock>();
    let mut unit = Reach::default();
    let mut sim = Running::new(unit.run(d_i, (l_o, c_o, s_o)));
    let n = |x: u8| U::<4>::from(x);
    d_o.set(Deep {
        pairs: [Pair { hi: n(1), lo: n(2) }, Pair { hi: n(3), lo: n(4) }],
        grid: [[n(5), n(6)], [n(7), n(8)]],
    });
    sim.cycle();
    sim.cycle();
    assert_eq!(l_i.get().raw(), 0x24, "the low nibbles");
    assert_eq!(c_i.get().raw(), 0x58, "the corners");
    assert_eq!(
        s_i.get().raw(),
        0x73,
        "grid[0x1][0usize] and pairs[1usize].hi"
    );
    let v = Reach::lowered("reach").verilog();
    for slice in ["d[27:24]", "d[19:16]", "d[15:12]", "d[3:0]", "d[7:4]"] {
        assert!(v.contains(slice), "no `{slice}` in:\n{v}");
    }
}
