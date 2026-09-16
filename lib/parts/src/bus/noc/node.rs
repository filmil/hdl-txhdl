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
use txhdl::comp::{join2, Rx, Tx, Unit};
use txhdl::{lower, Trace};

use super::pkt::Pkt;
use super::switch::Switch;

// begin{state}
/// A node of the lattice at `X`, `Y`: the request switch and the
/// response switch, each with its four links and its exit.
#[derive(Trace, Default)]
pub struct Node<
    const X: usize,
    const Y: usize,
    const XB: usize,
    const YB: usize,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    pub req: Switch<X, Y, XB, YB, A, D, S, I>,
    pub rsp: Switch<X, Y, XB, YB, A, D, S, I>,
}
// end{state}

// begin{run}
#[lower]
impl<
        const X: usize,
        const Y: usize,
        const XB: usize,
        const YB: usize,
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
    > Unit for Node<X, Y, XB, YB, A, D, S, I>
{
    #[allow(clippy::type_complexity)]
    async fn run(
        &mut self,
        (qni, qsi, qwi, qei, qxi, pni, psi, pwi, pei, pxi): (
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
        join2(
            self.req
                .run((qni, qsi, qwi, qei, qxi), (qno, qso, qwo, qeo, qxo)),
            self.rsp
                .run((pni, psi, pwi, pei, pxi), (pno, pso, pwo, peo, pxo)),
        )
        .await;
    }
}
// end{run}
