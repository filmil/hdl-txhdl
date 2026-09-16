// SPDX-License-Identifier: Apache-2.0
//! The five-port switch: one per virtual channel, and two of them
//! make a node.
//!
//! Four ports are the links to the neighbours and the fifth is the
//! exit. Routing is dimension order: a packet goes east or west until
//! it is in its destination's column, then north or south until it is
//! at its node, and then out of the exit. On a lattice that is
//! deadlock free with nothing else said, because a packet never turns
//! from Y back into X and so no cycle of waiting can form.
//!
//! One packet moves per cycle. Each input's direction and the room at
//! the output it wants are worked out first, so an input whose output
//! is full does not hold up the others, and the choice among those
//! that can move is by a fixed order that serves the four links
//! before the exit: a node does not starve the network in order to
//! feed itself.
use txhdl::comp::{mux, Clock, DefaultClock, Rx, Tx, Unit};
use txhdl::types::U;
use txhdl::{lower, Trace};

use super::pkt::Pkt;

/// Coordinates are carried at this width inside a switch, so that the
/// routing is one function whatever the lattice's shape.
pub const CW: usize = 8;

// begin{route}
/// Which port a packet at `cx`, `cy` leaves by: 4 east, 3 west, 2
/// south, 1 north, 0 the exit. Dimension order, X before Y.
#[lower]
fn route(dx: U<CW>, dy: U<CW>, cx: U<CW>, cy: U<CW>) -> U<3> {
    let east = dx > cx;
    let west = dx < cx;
    let here = !east & !west;
    let south = here & (dy > cy);
    let north = here & (dy < cy);
    mux(
        east,
        U::<3>::from(4u8),
        mux(
            west,
            U::<3>::from(3u8),
            mux(
                south,
                U::<3>::from(2u8),
                mux(north, U::<3>::from(1u8), U::<3>::from(0u8)),
            ),
        ),
    )
}
// end{route}

/// The room at the port a packet wants, as that port's `ready`.
#[lower]
fn room(
    r: U<3>,
    e: txhdl::types::Bit,
    w: txhdl::types::Bit,
    s: txhdl::types::Bit,
    n: txhdl::types::Bit,
    x: txhdl::types::Bit,
) -> txhdl::types::Bit {
    mux(r == 4, e, mux(r == 3, w, mux(r == 2, s, mux(r == 1, n, x))))
}

// begin{state}
/// A five-port switch at `X`, `Y` of a lattice. `XB` and `YB` are the
/// widths of a coordinate; `A`, `D`, `S` and `I` are the AXI link's.
#[derive(Trace, Default)]
pub struct Switch<
    const X: usize,
    const Y: usize,
    const XB: usize,
    const YB: usize,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {}
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
    > Unit for Switch<X, Y, XB, YB, A, D, S, I>
{
    async fn run(
        &mut self,
        (n_in, s_in, w_in, e_in, x_in): (
            Rx<Pkt<XB, YB, A, D, S, I>>,
            Rx<Pkt<XB, YB, A, D, S, I>>,
            Rx<Pkt<XB, YB, A, D, S, I>>,
            Rx<Pkt<XB, YB, A, D, S, I>>,
            Rx<Pkt<XB, YB, A, D, S, I>>,
        ),
        (n_out, s_out, w_out, e_out, x_out): (
            Tx<Pkt<XB, YB, A, D, S, I>>,
            Tx<Pkt<XB, YB, A, D, S, I>>,
            Tx<Pkt<XB, YB, A, D, S, I>>,
            Tx<Pkt<XB, YB, A, D, S, I>>,
            Tx<Pkt<XB, YB, A, D, S, I>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let cx = U::<CW>::from(X as u32);
            let cy = U::<CW>::from(Y as u32);
            let (re, rw, rs, rn, rx) = (
                e_out.ready(),
                w_out.ready(),
                s_out.ready(),
                n_out.ready(),
                x_out.ready(),
            );
            // What each input offers, where it would leave by, and
            // whether that port has room for it.
            let pn = n_in.head();
            let dn = route(pn.dx.resize::<CW>(), pn.dy.resize::<CW>(), cx, cy);
            let kn = n_in.peek().is_some() & room(dn, re, rw, rs, rn, rx);
            let ps = s_in.head();
            let ds = route(ps.dx.resize::<CW>(), ps.dy.resize::<CW>(), cx, cy);
            let ks = s_in.peek().is_some() & room(ds, re, rw, rs, rn, rx);
            let pw = w_in.head();
            let dw = route(pw.dx.resize::<CW>(), pw.dy.resize::<CW>(), cx, cy);
            let kw = w_in.peek().is_some() & room(dw, re, rw, rs, rn, rx);
            let pe = e_in.head();
            let de = route(pe.dx.resize::<CW>(), pe.dy.resize::<CW>(), cx, cy);
            let ke = e_in.peek().is_some() & room(de, re, rw, rs, rn, rx);
            let px = x_in.head();
            let dx = route(px.dx.resize::<CW>(), px.dy.resize::<CW>(), cx, cy);
            let kx = x_in.peek().is_some() & room(dx, re, rw, rs, rn, rx);
            // One packet moves, the links before the exit.
            let gn = kn;
            let gs = ks & !gn;
            let gw = kw & !gn & !gs;
            let ge = ke & !gn & !gs & !gw;
            let gx = kx & !gn & !gs & !gw & !ge;
            let _ = n_in.recv_if(gn);
            let _ = s_in.recv_if(gs);
            let _ = w_in.recv_if(gw);
            let _ = e_in.recv_if(ge);
            let _ = x_in.recv_if(gx);
            // The packet that moves, and the port it leaves by.
            let p = mux(gn, pn, mux(gs, ps, mux(gw, pw, mux(ge, pe, px))));
            let d = mux(gn, dn, mux(gs, ds, mux(gw, dw, mux(ge, de, dx))));
            let go = gn | gs | gw | ge | gx;
            if (go & (d == 4)).to_bool() {
                e_out.send(p);
            }
            if (go & (d == 3)).to_bool() {
                w_out.send(p);
            }
            if (go & (d == 2)).to_bool() {
                s_out.send(p);
            }
            if (go & (d == 1)).to_bool() {
                n_out.send(p);
            }
            if (go & (d == 0)).to_bool() {
                x_out.send(p);
            }
        }
    }
}
// end{run}
