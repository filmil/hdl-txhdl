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
    #[default]
    Aw,
    Ar,
    W,
    B,
    R,
}

/// Which virtual channel a packet belongs to. A request never waits
/// behind a response and a response never waits behind a request,
/// which is what makes the two of them separate links.
#[derive(ValueDerive, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum Vc {
    #[default]
    Request,
    Response,
}

/// A packet. `XB` and `YB` are the widths of a coordinate, so a
/// lattice is `1 << XB` by `1 << YB` nodes at most.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct Pkt<
    const XB: usize,
    const YB: usize,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    /// Where it is going, and where it came from, so that an answer
    /// knows its way back without anything keeping a table.
    pub dx: U<XB>,
    pub dy: U<YB>,
    pub sx: U<XB>,
    pub sy: U<YB>,
    /// Which channel's beat this is, and the identifier it carries.
    pub chan: Chan,
    pub id: U<I>,
    /// An address phase: `Aw` and `Ar`.
    pub addr: U<A>,
    pub len: U<8>,
    pub size: U<3>,
    pub burst: BurstKind,
    pub lock: Bit,
    pub cache: U<4>,
    pub prot: U<3>,
    pub qos: U<4>,
    pub region: U<4>,
    /// A data beat: `W` and `R`.
    pub data: U<D>,
    pub strb: U<S>,
    pub last: Bit,
    /// A response: `B` and `R`.
    pub resp: Resp,
}
// end{pkt}
