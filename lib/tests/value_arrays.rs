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
