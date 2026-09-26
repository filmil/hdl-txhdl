// SPDX-License-Identifier: Apache-2.0
//! A lattice of network nodes as one lowered unit (issue 635).
//!
//! `Mesh<3, 2, 6, ..>` from `bus::noc` is six nodes, three wide and
//! two high, held as one field of `Units` and joined by index. Every
//! node is one type, since its place is two inputs the mesh ties to
//! constants rather than two type parameters. The bench has every
//! node send a request to each of the other five, and a response to
//! the node at the opposite corner of the lattice, and checks that
//! each packet leaves by the exit of the node it was sent to and
//! that none is lost. The netlist of the mesh, six node instances and
//! the channels between them, is checked against the run under nvc
//! and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, DefaultClock, Running, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::noc::mesh::Mesh;
use txhdl_parts::bus::noc::pkt::{Chan, Pkt};

/// Three nodes wide and two high; a column fits in two bits and a row
/// in one. The link's widths are small, since only the routing is
/// being shown: an eight-bit address and eight-bit data.
const W: usize = 3;
const H: usize = 2;
const N: usize = 6;
type P = Pkt<2, 1, 8, 8, 1, 2>;
type M = Mesh<W, H, N, 2, 1, 8, 8, 1, 2>;

/// A packet from node `from` to node `to`; its address says who sent
/// it, so the exit it leaves by can be checked against it.
fn pkt(from: usize, to: usize, chan: Chan) -> P {
    P {
        dx: U::from(to % W),
        dy: U::from(to / W),
        sx: U::from(from % W),
        sy: U::from(from / W),
        chan,
        addr: U::from(from),
        last: Bit::One,
        ..P::default()
    }
}

/// `N` channels, as the arrays of their two ends.
fn chans_of() -> ([Tx<P>; N], [Rx<P>; N]) {
    let (tx, rx): (Vec<_>, Vec<_>) =
        (0..N).map(|_| chan::<P, DefaultClock>()).unzip();
    let ok = "N channels were made";
    (tx.try_into().ok().expect(ok), rx.try_into().ok().expect(ok))
}

fn main() {
    let (qi_tx, qi_rx) = chans_of();
    let (pi_tx, pi_rx) = chans_of();
    let (qo_tx, qo_rx) = chans_of();
    let (po_tx, po_rx) = chans_of();
    let mut mesh = M::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        for k in 0..N {
            w.add(&format!("qx_in_{k}"), &qi_rx[k]);
            w.add(&format!("px_in_{k}"), &pi_rx[k]);
        }
        w.add("mesh", &mesh);
        for k in 0..N {
            w.add(&format!("qx_out_{k}"), &qo_rx[k]);
            w.add(&format!("px_out_{k}"), &po_rx[k]);
        }
        w.start();
    }
    let mut sim = Running::new(mesh.run((qi_rx, pi_rx), (qo_tx, po_tx)));
    // What each node has still to send: requests to the other five in
    // index order, and one response to the opposite corner.
    let mut reqs: Vec<Vec<usize>> = (0..N)
        .map(|k| (0..N).filter(|j| *j != k).collect())
        .collect();
    let mut rsps: Vec<Option<usize>> =
        (0..N).map(|k| Some(N - 1 - k)).collect();
    // What each exit has seen, as the senders' indices.
    let mut got_q: Vec<Vec<usize>> = vec![Vec::new(); N];
    let mut got_p: Vec<Vec<usize>> = vec![Vec::new(); N];
    for t in 0..60 {
        for k in 0..N {
            if let (Some(&to), true) =
                (reqs[k].first(), qi_tx[k].ready().to_bool())
            {
                qi_tx[k].send(pkt(k, to, Chan::Ar));
                reqs[k].remove(0);
            }
            if let (Some(to), true) = (rsps[k], pi_tx[k].ready().to_bool()) {
                pi_tx[k].send(pkt(k, to, Chan::R));
                rsps[k] = None;
            }
        }
        for j in 0..N {
            if let Some(p) = qo_rx[j].recv() {
                let from = p.addr.raw() as usize;
                println!("t={t:2} request  {from} -> {j}");
                assert_eq!(p.dx.raw() as usize + W * p.dy.raw() as usize, j);
                got_q[j].push(from);
            }
            if let Some(p) = po_rx[j].recv() {
                let from = p.addr.raw() as usize;
                println!("t={t:2} response {from} -> {j}");
                assert_eq!(N - 1 - from, j, "a response left at {j}");
                got_p[j].push(from);
            }
        }
        sim.cycle();
    }
    // Every node heard once from every other, on each channel it was
    // sent on, and nothing else arrived.
    for j in 0..N {
        let mut q = got_q[j].clone();
        q.sort_unstable();
        let want: Vec<usize> = (0..N).filter(|k| *k != j).collect();
        assert_eq!(q, want, "the requests node {j} received");
        assert_eq!(got_p[j], vec![N - 1 - j], "node {j}'s response");
    }
    println!("all {} requests and {N} responses arrived", N * (N - 1));
    stop();
    let net = M::lowered("mesh");
    let names: Vec<&str> =
        net.instances.iter().map(|i| i.name.as_str()).collect();
    println!("instances: {names:?}");
    txhdl::netlist::write_vhdl_from_env(&net);
}
