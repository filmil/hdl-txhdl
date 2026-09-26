// SPDX-License-Identifier: Apache-2.0
//! A node: a switch per virtual channel.
//!
//! The two switches share nothing, which is the point of a virtual
//! channel. A request held up behind a full buffer cannot hold up the
//! response that would empty it, because the response is on the other
//! switch and its buffers are its own.
//!
//! The twenty ports are written out one by one because a lowered
//! unit's ports are a positional tuple of `Rx` and `Tx` and nothing
//! else: a named type for a side, or a tuple of tuples, is refused.
//! The naming is `q` for the request channel and `p` for the
//! response, then the port, then `i` or `o`.
use txhdl::comp::{join2, In, Rx, Tx, Unit};
use txhdl::types::U;
use txhdl::{lower, Trace};

use super::pkt::Pkt;
use super::switch::Switch;

// begin{state}
/// A node of the lattice: the request switch and the response switch,
/// each with its four links and its exit, both at the column and row
/// its inputs `col` and `row` say (issue 635).
///
/// The widths are the AXI link's, and they are the same everywhere in
/// the network: `A` is the address width, `D` the data width, `S` the
/// strobe width, which is `D / 8`, and `I` the identifier width. `XB`
/// and `YB` are the widths of a coordinate, so a lattice is `1 << XB`
/// by `1 << YB` nodes at most.
#[derive(Trace, Default)]
pub struct Node<
    const XB: usize,
    const YB: usize,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    /// What a host sends and a peripheral receives.
    pub req: Switch<XB, YB, A, D, S, I>,
    /// What a peripheral sends and a host receives.
    pub rsp: Switch<XB, YB, A, D, S, I>,
}
// end{state}

// begin{run}
#[lower]
impl<
        const XB: usize,
        const YB: usize,
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
    > Unit for Node<XB, YB, A, D, S, I>
{
    #[allow(clippy::type_complexity)]
    async fn run(
        &mut self,
        (col, row, qni, qsi, qwi, qei, qxi, pni, psi, pwi, pei, pxi): (
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
        ),
        (qno, qso, qwo, qeo, qxo, pno, pso, pwo, peo, pxo): (
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
        ),
    ) {
        // Both switches are at the node's place.
        let col2 = col.clone();
        let row2 = row.clone();
        join2(
            self.req.run(
                (col, row, qni, qsi, qwi, qei, qxi),
                (qno, qso, qwo, qeo, qxo),
            ),
            self.rsp.run(
                (col2, row2, pni, psi, pwi, pei, pxi),
                (pno, pso, pwo, peo, pxo),
            ),
        )
        .await;
    }
}
// end{run}
