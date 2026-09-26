// SPDX-License-Identifier: Apache-2.0
//! An array of child units, in a lowered unit of units (issue 635).
//!
//! `Lanes<N>` holds `N` lanes as one field, `lanes: Units<Lane, N>`,
//! joined in a ring of channels: each lane sends its input to the lane
//! on its right, keeps what the lane on its left sent, and puts out its
//! input plus that. The parent's `run` makes the ring's `N` channels
//! with `chans`, hands its array ports out by index with `Ends::from`,
//! under names of their own, and joins the lanes with `join_all`, lane
//! `i` taking its own input and output, the channel it sends on, `i`,
//! and the one it receives from, `(i + N - 1) % N`. The ring is of
//! channels rather than wires because a channel's word is seen the
//! step after it is sent whichever lane runs first, where a wire read
//! in the step it is driven depends on the order. The lowering unrolls
//! the join into an instance per lane, `lanes_0` to `lanes_{N-1}`, and
//! the nets `ring_0` onward; the netlist of `Lanes<3>` is checked
//! against the run under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chans, join_all, signal, Clock, DefaultClock, Ends, In, Out, Reg, Running,
    Rx, Tx, Unit, Units,
};
use txhdl::types::U;
use txhdl::{lower, with, Trace};

// begin{unit}
/// One lane of the ring.
#[derive(Trace, Default)]
pub struct Lane {
    /// What the lane on the left sent last.
    pub got: Reg<U<8>>,
}

#[lower]
impl Unit for Lane {
    async fn run(
        &mut self,
        (x, left): (In<U<8>>, Rx<U<8>>),
        (y, right): (Out<U<8>>, Tx<U<8>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let v = x.get();
            if right.ready().to_bool() {
                right.send(v);
            }
            let has = left.peek().is_some();
            let n = left.head();
            let _ = left.recv_if(has);
            with!(self <= { has ? got: n });
            y.set(self.got + v);
        }
    }
}

/// `N` lanes in a ring.
#[derive(Trace, Default)]
pub struct Lanes<const N: usize> {
    /// The lanes, `lanes_0` onward in the netlist.
    pub lanes: Units<Lane, N>,
}

#[lower]
impl<const N: usize> Unit for Lanes<N> {
    async fn run(&mut self, xs: [In<U<8>>; N], ys: [Out<U<8>>; N]) {
        let (mut ring_tx, mut ring_rx) = chans::<U<8>, DefaultClock, N>();
        let mut x_ends = Ends::from(xs);
        let mut y_ends = Ends::from(ys);
        join_all(self.lanes.iter_mut().enumerate().map(|(i, lane)| {
            lane.run(
                (x_ends.take(i), ring_rx.take((i + N - 1) % N)),
                (y_ends.take(i), ring_tx.take(i)),
            )
        }))
        .await;
    }
}
// end{unit}

fn main() {
    let (x0_out, x0) = signal::<U<8>, DefaultClock>();
    let (x1_out, x1) = signal::<U<8>, DefaultClock>();
    let (x2_out, x2) = signal::<U<8>, DefaultClock>();
    let (y0_out, y0) = signal::<U<8>, DefaultClock>();
    let (y1_out, y1) = signal::<U<8>, DefaultClock>();
    let (y2_out, y2) = signal::<U<8>, DefaultClock>();
    let mut unit = Lanes::<3>::default();
    let (xs, ys) = ([x0, x1, x2], [y0, y1, y2]);
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("xs_0", &xs[0]);
        w.add("xs_1", &xs[1]);
        w.add("xs_2", &xs[2]);
        w.add("lanes", &unit);
        w.add("ys_0", &ys[0]);
        w.add("ys_1", &ys[1]);
        w.add("ys_2", &ys[2]);
        w.start();
    }
    let drive = [x0_out, x1_out, x2_out];
    let read = ys.clone();
    let mut sim = Running::new(unit.run(xs, [y0_out, y1_out, y2_out]));
    // Lane k is given 10k + t at cycle t.
    let mut seen: Vec<Vec<u128>> = Vec::new();
    for t in 0..8u8 {
        for (k, d) in drive.iter().enumerate() {
            d.set(U::<8>::from(10 * k as u8 + t));
        }
        sim.cycle();
        let got: Vec<u128> = read.iter().map(|y| y.get().raw()).collect();
        println!("t={t} ys {got:?}");
        seen.push(got);
    }
    // Once the ring is full every lane is alike, whichever lane ran
    // first in a step: its own input plus what its left neighbour was
    // given two cycles before, one for the channel and one for `got`.
    let x = |k: usize, t: usize| (10 * k + t) as u128;
    for (t, ys) in seen.iter().enumerate().skip(2) {
        for (k, y) in ys.iter().enumerate() {
            assert_eq!(*y, x(k, t) + x((k + 2) % 3, t - 2), "lane {k} at {t}");
        }
    }
    stop();
    let net = Lanes::<3>::lowered("lanes");
    let names: Vec<&str> =
        net.instances.iter().map(|i| i.name.as_str()).collect();
    println!("instances: {names:?}");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
