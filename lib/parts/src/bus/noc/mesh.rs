// SPDX-License-Identifier: Apache-2.0
//! Wiring a lattice, two ways.
//!
//! `Mesh` is a lattice as one unit of units, which lowers to a module
//! of `W * H` node instances (issue 635).
//!
//! `lattice` is simulation-side help, like `bus::axi::sim`, and does
//! not lower. It makes the channels of a lattice of nodes
//! and hands out the ends, so that a design says which nodes it wants
//! and what attaches to each exit, rather than naming eighty channel
//! ends by hand.
use txhdl::comp::{
    chan, chans, join_all, tie, DefaultClock, Ends, In, Rx, Tx, Unit, Units,
};
use txhdl::types::U;
use txhdl::{lower, Trace};

use super::node::Node;
use super::pkt::Pkt;

/// What a node reads: its column and row, held at constants,
/// then north, south, west, east and the exit, for the request channel
/// and then the response channel (issue 635).
pub type Ins<
    const XB: usize,
    const YB: usize,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> = (
    In<U<XB>>,
    In<U<YB>>,
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
///
/// The widths are the AXI link's, and they are the same everywhere in
/// the network: `A` is the address width, `D` the data width, `S` the
/// strobe width, which is `D / 8`, and `I` the identifier width. `XB`
/// and `YB` are the widths of a coordinate, so a lattice is `1 << XB`
/// by `1 << YB` nodes at most.
pub struct Exit<
    const XB: usize,
    const YB: usize,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    /// Requests into the node, which a host's bridge drives.
    pub q_in: Tx<Pkt<XB, YB, A, D, S, I>>,
    /// Requests out of the node, which a peripheral's bridge reads.
    pub q_out: Rx<Pkt<XB, YB, A, D, S, I>>,
    /// Responses into the node, which a peripheral's bridge drives.
    pub p_in: Tx<Pkt<XB, YB, A, D, S, I>>,
    /// Responses out of the node, which a host's bridge reads.
    pub p_out: Rx<Pkt<XB, YB, A, D, S, I>>,
}

/// A lattice, wired: per node in row-major order, the ends its `run`
/// takes, and the ends whatever attaches to its exit holds.
///
/// The widths are the AXI link's, and they are the same everywhere in
/// the network: `A` is the address width, `D` the data width, `S` the
/// strobe width, which is `D / 8`, and `I` the identifier width. `XB`
/// and `YB` are the widths of a coordinate, so a lattice is `1 << XB`
/// by `1 << YB` nodes at most.
pub struct Lattice<
    const XB: usize,
    const YB: usize,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    /// What each node's `run` reads, in row-major order.
    pub ins: Vec<Ins<XB, YB, A, D, S, I>>,
    /// What each node's `run` drives, in the same order.
    pub outs: Vec<Outs<XB, YB, A, D, S, I>>,
    /// What attaches at each node's exit, in the same order.
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
                tie(U::<XB>::from(k % w)),
                tie(U::<YB>::from(k / w)),
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

/// The node to the west of node `i` on a lattice `w` wide, the row's
/// last when `i` is its first.
pub const fn west(i: usize, w: usize) -> usize {
    i - i % w + (i % w + w - 1) % w
}

/// The node to the east of node `i` on a lattice `w` wide, the row's
/// first when `i` is its last.
pub const fn east(i: usize, w: usize) -> usize {
    i - i % w + (i % w + 1) % w
}

// begin{mesh}
/// A lattice of `W` by `H` nodes as one unit, which lowers: `N` nodes
/// in row-major order, `N` being `W * H`, and each at the place its
/// index says, held at constants with `tie` (issue 635).
///
/// Its ports are the exits, each an array of `N`: `qx_in` the requests
/// into each node and `px_in` the responses, `qx_out` and `px_out` what
/// leaves by each. The links are one array of channels per direction
/// of travel and per virtual channel, a node's output to a direction
/// being channel `i` of that direction's array and its neighbour's
/// input from the opposite side.
///
/// A link at an edge wraps round to the other end of its row or column
/// rather than going nowhere, so that every channel has one driver and
/// one reader. Routing is dimension order, so no packet for a place on
/// the lattice ever takes one: nothing leaves the east edge eastward,
/// since no column is east of it.
///
/// The widths are the AXI link's, and they are the same everywhere in
/// the network: `A` is the address width, `D` the data width, `S` the
/// strobe width, which is `D / 8`, and `I` the identifier width. `XB`
/// and `YB` are the widths of a coordinate.
#[derive(Trace)]
pub struct Mesh<
    const W: usize,
    const H: usize,
    const N: usize,
    const XB: usize,
    const YB: usize,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    /// The nodes, `nodes_0` onward in the netlist, row major.
    pub nodes: Units<Node<XB, YB, A, D, S, I>, N>,
}

/// A mesh whose count is not its width times its height is refused
/// here, since Rust cannot yet compute `N` from `W` and `H`.
impl<
        const W: usize,
        const H: usize,
        const N: usize,
        const XB: usize,
        const YB: usize,
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
    > Default for Mesh<W, H, N, XB, YB, A, D, S, I>
{
    fn default() -> Self {
        assert_eq!(N, W * H, "a mesh of {W} by {H} is {} nodes", W * H);
        Mesh {
            nodes: Units::default(),
        }
    }
}

#[lower]
impl<
        const W: usize,
        const H: usize,
        const N: usize,
        const XB: usize,
        const YB: usize,
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
    > Unit for Mesh<W, H, N, XB, YB, A, D, S, I>
{
    async fn run(
        &mut self,
        (qx_in, px_in): (
            [Rx<Pkt<XB, YB, A, D, S, I>>; N],
            [Rx<Pkt<XB, YB, A, D, S, I>>; N],
        ),
        (qx_out, px_out): (
            [Tx<Pkt<XB, YB, A, D, S, I>>; N],
            [Tx<Pkt<XB, YB, A, D, S, I>>; N],
        ),
    ) {
        // The links, by the way they travel: up, down, east and west,
        // for requests and then for responses.
        let (mut qu_tx, mut qu_rx) =
            chans::<Pkt<XB, YB, A, D, S, I>, DefaultClock, N>();
        let (mut qd_tx, mut qd_rx) =
            chans::<Pkt<XB, YB, A, D, S, I>, DefaultClock, N>();
        let (mut qe_tx, mut qe_rx) =
            chans::<Pkt<XB, YB, A, D, S, I>, DefaultClock, N>();
        let (mut qw_tx, mut qw_rx) =
            chans::<Pkt<XB, YB, A, D, S, I>, DefaultClock, N>();
        let (mut pu_tx, mut pu_rx) =
            chans::<Pkt<XB, YB, A, D, S, I>, DefaultClock, N>();
        let (mut pd_tx, mut pd_rx) =
            chans::<Pkt<XB, YB, A, D, S, I>, DefaultClock, N>();
        let (mut pe_tx, mut pe_rx) =
            chans::<Pkt<XB, YB, A, D, S, I>, DefaultClock, N>();
        let (mut pw_tx, mut pw_rx) =
            chans::<Pkt<XB, YB, A, D, S, I>, DefaultClock, N>();
        let mut qxi = Ends::from(qx_in);
        let mut pxi = Ends::from(px_in);
        let mut qxo = Ends::from(qx_out);
        let mut pxo = Ends::from(px_out);
        // Node `i` reads from the north what the node above sent down,
        // from the south what the node below sent up, from the west
        // what the node to its west sent east, and from the east what
        // the node to its east sent west.
        join_all(self.nodes.iter_mut().enumerate().map(|(i, node)| {
            node.run(
                (
                    tie(U::<XB>::from(i % W)),
                    tie(U::<YB>::from(i / W)),
                    qd_rx.take((i + N - W) % N),
                    qu_rx.take((i + W) % N),
                    qe_rx.take(west(i, W)),
                    qw_rx.take(east(i, W)),
                    qxi.take(i),
                    pd_rx.take((i + N - W) % N),
                    pu_rx.take((i + W) % N),
                    pe_rx.take(west(i, W)),
                    pw_rx.take(east(i, W)),
                    pxi.take(i),
                ),
                (
                    qu_tx.take(i),
                    qd_tx.take(i),
                    qw_tx.take(i),
                    qe_tx.take(i),
                    qxo.take(i),
                    pu_tx.take(i),
                    pd_tx.take(i),
                    pw_tx.take(i),
                    pe_tx.take(i),
                    pxo.take(i),
                ),
            )
        }))
        .await;
    }
}
// end{mesh}
