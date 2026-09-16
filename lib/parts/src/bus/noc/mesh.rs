// SPDX-License-Identifier: Apache-2.0
//! Wiring a lattice: simulation-side help, like `bus::axi::sim`.
//!
//! Nothing here lowers. It makes the channels of a lattice of nodes
//! and hands out the ends, so that a design says which nodes it wants
//! and what attaches to each exit, rather than naming eighty channel
//! ends by hand. A design that is to be synthesised writes the same
//! wiring as a unit of units and lets the lowering make the netlist.
use txhdl::comp::{chan, DefaultClock, Rx, Tx};

use super::pkt::Pkt;

/// What a node reads: north, south, west, east and the exit, for the
/// request channel and then the response channel.
pub type Ins<
    const XB: usize,
    const YB: usize,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> = (
    Rx<Pkt<XB, YB, A, D, S, I>>,
    Rx<Pkt<XB, YB, A, D, S, I>>,
    Rx<Pkt<XB, YB, A, D, S, I>>,
    Rx<Pkt<XB, YB, A, D, S, I>>,
    Rx<Pkt<XB, YB, A, D, S, I>>,
    Rx<Pkt<XB, YB, A, D, S, I>>,
    Rx<Pkt<XB, YB, A, D, S, I>>,
    Rx<Pkt<XB, YB, A, D, S, I>>,
    Rx<Pkt<XB, YB, A, D, S, I>>,
    Rx<Pkt<XB, YB, A, D, S, I>>,
);

/// What a node drives, in the same order.
pub type Outs<
    const XB: usize,
    const YB: usize,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> = (
    Tx<Pkt<XB, YB, A, D, S, I>>,
    Tx<Pkt<XB, YB, A, D, S, I>>,
    Tx<Pkt<XB, YB, A, D, S, I>>,
    Tx<Pkt<XB, YB, A, D, S, I>>,
    Tx<Pkt<XB, YB, A, D, S, I>>,
    Tx<Pkt<XB, YB, A, D, S, I>>,
    Tx<Pkt<XB, YB, A, D, S, I>>,
    Tx<Pkt<XB, YB, A, D, S, I>>,
    Tx<Pkt<XB, YB, A, D, S, I>>,
    Tx<Pkt<XB, YB, A, D, S, I>>,
);

/// What attaches at a node's exit: a host bridge drives `q_in` and
/// reads `p_out`, and a peripheral bridge drives `p_in` and reads
/// `q_out`.
pub struct Exit<
    const XB: usize,
    const YB: usize,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    pub q_in: Tx<Pkt<XB, YB, A, D, S, I>>,
    pub q_out: Rx<Pkt<XB, YB, A, D, S, I>>,
    pub p_in: Tx<Pkt<XB, YB, A, D, S, I>>,
    pub p_out: Rx<Pkt<XB, YB, A, D, S, I>>,
}

/// A lattice, wired: per node in row-major order, the ends its `run`
/// takes, and the ends whatever attaches to its exit holds.
pub struct Lattice<
    const XB: usize,
    const YB: usize,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    pub ins: Vec<Ins<XB, YB, A, D, S, I>>,
    pub outs: Vec<Outs<XB, YB, A, D, S, I>>,
    pub exits: Vec<Exit<XB, YB, A, D, S, I>>,
}

/// Make the channels of a `w` by `h` lattice and hand out the ends.
/// A link at the edge is made as any other and goes nowhere, so a
/// node at a corner is the same unit as one in the middle.
///
/// Ports are numbered north, south, west, east, exit, and a link
/// leaves one node's north and enters its neighbour's south, so a
/// node's output to a direction is the neighbour's input from the
/// opposite one.
#[allow(clippy::type_complexity)]
pub fn lattice<
    const XB: usize,
    const YB: usize,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
>(
    w: usize,
    h: usize,
) -> Lattice<XB, YB, A, D, S, I> {
    let n = w * h;
    let make = || chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
    // Every input of every node, on both virtual channels: the reader
    // goes to the node, the driver to whatever feeds it.
    let idx = |k: usize, v: usize, p: usize| (k * 2 + v) * 5 + p;
    let mut itx = Vec::new();
    let mut irx = Vec::new();
    for _ in 0..n * 2 * 5 {
        let (a, b) = make();
        itx.push(Some(a));
        irx.push(Some(b));
    }
    // Each node's exit output, whose reader is the caller's.
    let mut etx = Vec::new();
    let mut erx = Vec::new();
    for _ in 0..n * 2 {
        let (a, b) = make();
        etx.push(Some(a));
        erx.push(Some(b));
    }
    // A link at the edge drives a channel nobody reads.
    let mut stx = Vec::new();
    for _ in 0..n * 2 * 4 {
        let (a, _b) = make();
        stx.push(Some(a));
    }

    let ins: Vec<Ins<XB, YB, A, D, S, I>> = (0..n)
        .map(|k| {
            let mut take = |v, p| irx[idx(k, v, p)].take().unwrap();
            (
                take(0, 0),
                take(0, 1),
                take(0, 2),
                take(0, 3),
                take(0, 4),
                take(1, 0),
                take(1, 1),
                take(1, 2),
                take(1, 3),
                take(1, 4),
            )
        })
        .collect();

    let at = |x: i32, y: i32| -> Option<usize> {
        (x >= 0 && y >= 0 && x < w as i32 && y < h as i32)
            .then(|| y as usize * w + x as usize)
    };
    let mut outs = Vec::new();
    for k in 0..n {
        let (x, y) = ((k % w) as i32, (k / w) as i32);
        // Each way out, and the port it enters the neighbour by.
        let ways = [
            (at(x, y - 1), 1usize),
            (at(x, y + 1), 0),
            (at(x - 1, y), 3),
            (at(x + 1, y), 2),
        ];
        let mut side = |v: usize| {
            let mut t = Vec::new();
            for (d, (dst, port)) in ways.iter().enumerate() {
                t.push(match dst {
                    Some(j) => itx[idx(*j, v, *port)].take().unwrap(),
                    None => stx[(k * 2 + v) * 4 + d].take().unwrap(),
                });
            }
            t
        };
        let mut q = side(0);
        let mut p = side(1);
        outs.push((
            q.remove(0),
            q.remove(0),
            q.remove(0),
            q.remove(0),
            etx[k * 2].take().unwrap(),
            p.remove(0),
            p.remove(0),
            p.remove(0),
            p.remove(0),
            etx[k * 2 + 1].take().unwrap(),
        ));
    }

    let exits = (0..n)
        .map(|k| Exit {
            q_in: itx[idx(k, 0, 4)].take().unwrap(),
            q_out: erx[k * 2].take().unwrap(),
            p_in: itx[idx(k, 1, 4)].take().unwrap(),
            p_out: erx[k * 2 + 1].take().unwrap(),
        })
        .collect();

    Lattice { ins, outs, exits }
}
