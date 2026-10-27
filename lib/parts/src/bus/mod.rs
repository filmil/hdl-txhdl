// SPDX-License-Identifier: Apache-2.0
//! Buses: what a channel is not. A channel carries one transaction in
//! one direction; a bus is several channels that only mean something
//! together, with a protocol over them. Each bus here is written so
//! that a client waits on whole transactions and writes none of the
//! protocol: no beat counting, no identifier, no channel named.
//!
//! - [`axi`]: an AXI4 link, five channels behind two ends.
//! - [`axi_lite`]: AXI4-Lite, and the bridge from an AXI4 link to
//!   small peripherals on it.
//! - [`axi_pins`]: a host's AXI4 pins, joined to the link's channels.
//! - [`axi_per_pins`]: the link's peripheral end, joined to an AXI4
//!   peripheral's pins.
//! - [`arbiter`]: several hosts merged onto one link, and
//!   [`router`]: one host fanned out to several peripherals.
//! - [`exmon`]: the exclusive monitor, which makes `lr.w`, `sc.w`
//!   and the AMOs atomic across the hosts on one memory.
//! - [`lite_split`]: one AXI-Lite slot split in two by an address bit.
//! - [`noc`]: a network on chip, and the bridges that put an AXI link
//!   across it.
//! - [`wb`]: a bridge from a link's peripheral end to a pipelined
//!   Wishbone bus.
/// The markers the bus structs give a type parameter by, declared once
/// so the router and the AXI-Lite bridge share them (#1914): `Map<M>`,
/// the address map a part decodes with.
pub mod named {
    txhdl::named_markers!(pub Map);
}
pub use named::{IsMap, Map};

// First, and with `#[macro_use]`, so the by-name macros of the AXI4
// structs (`AxiHost![..]` and the rest) are in scope in the modules
// after it (#1915).
#[macro_use]
pub mod axi;
#[macro_use]
pub mod arbiter;
#[macro_use]
pub mod axi_lite;
pub mod axi_per_pins;
pub mod axi_pins;
pub mod exmon;
#[macro_use]
pub mod lite_split;
// Before `noc`, whose tests write the router by name.
#[macro_use]
pub mod router;
pub mod noc;
pub mod wb;
