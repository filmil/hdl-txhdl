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
//! - [`arbiter`]: several hosts merged onto one link, and
//!   [`router`]: one host fanned out to several peripherals.
pub mod arbiter;
pub mod axi;
pub mod axi_lite;
pub mod axi_pins;
pub mod noc;
pub mod router;
pub mod wb;
