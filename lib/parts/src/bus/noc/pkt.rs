// SPDX-License-Identifier: Apache-2.0
//! What a link carries: one beat of one AXI channel, addressed to a
//! node and stamped with the node it came from.
//!
//! A packet is one shape, as wide as the widest beat needs, for the
//! same reason the GPU's instruction is: every field a step reads has
//! to be at a fixed place, and a link that carried a header and then
//! a body would want a sequencer at both ends before anything else
//! worked. A read address beat leaves the data and the strobe at
//! zero; a write response leaves the address at zero. [`Chan`] says
//! which fields mean anything.
use txhdl::types::{Bit, U};
use txhdl::{Transaction as TransactionDerive, Value as ValueDerive};

use crate::bus::axi::{BurstKind, Resp};

// begin{pkt}
/// Which of AXI's five channels a packet carries.
#[derive(ValueDerive, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum Chan {
    /// A write address phase. In this network it carries its single
    /// data beat with it, so there is no separate `W`.
    #[default]
    Aw,
    /// A read address phase.
    Ar,
    /// A write: an address phase and its one beat, together.
    W,
    /// A write response.
    B,
    /// A read data beat.
    R,
}

/// Which virtual channel a packet belongs to. A request never waits
/// behind a response and a response never waits behind a request,
/// which is what makes the two of them separate links.
#[derive(ValueDerive, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum Vc {
    /// What a host sends and a peripheral receives.
    #[default]
    Request,
    /// What a peripheral sends and a host receives.
    Response,
}

/// A packet: one beat of one AXI channel, addressed to a node.
///
/// The widths are the AXI link's, and they are the same everywhere in
/// the network: `A` is the address width, `D` the data width, `S` the
/// strobe width, which is `D / 8`, and `I` the identifier width. `XB`
/// and `YB` are the widths of a coordinate, so a lattice is `1 << XB`
/// by `1 << YB` nodes at most.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct Pkt<
    const XB: usize,
    const YB: usize,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    // Where it is going, and where it came from, so that an answer
    // knows its way back without anything keeping a table.
    /// The column of the node it is going to.
    pub dx: U<XB>,
    /// The row of the node it is going to.
    pub dy: U<YB>,
    /// The column of the node it came from.
    pub sx: U<XB>,
    /// The row of the node it came from.
    pub sy: U<YB>,
    /// Which channel's beat this is, and so which fields below mean
    /// anything.
    pub chan: Chan,
    /// The AXI identifier the beat carries.
    pub id: U<I>,
    // An address phase: `Aw` and `Ar`.
    /// The byte address the burst starts at.
    pub addr: U<A>,
    /// Beats in the burst, less one, as AXI counts them.
    pub len: U<8>,
    /// The log of the bytes in a beat, as AXI states it.
    pub size: U<3>,
    /// Fixed, incrementing or wrapping.
    pub burst: BurstKind,
    /// AXI's lock, carried and not acted on.
    pub lock: Bit,
    /// AXI's cache hints, carried and not acted on.
    pub cache: U<4>,
    /// AXI's protection bits, carried and not acted on.
    pub prot: U<3>,
    /// AXI's quality of service, carried and not acted on.
    pub qos: U<4>,
    /// AXI's region, carried and not acted on.
    pub region: U<4>,
    // A data beat: `W` and `R`.
    /// The beat's data, on `W` and `R`.
    pub data: U<D>,
    /// Which lanes of `data` a write covers.
    pub strb: U<S>,
    /// Whether this is the burst's final beat.
    pub last: Bit,
    // A response: `B` and `R`.
    /// The answer, on `B` and `R`.
    pub resp: Resp,
}
// end{pkt}
