// SPDX-License-Identifier: Apache-2.0
//! Buses: what a channel is not. A channel carries one transaction in
//! one direction; a bus is several channels that only mean something
//! together, with a protocol over them. Each bus here is written so
//! that a client waits on whole transactions and writes none of the
//! protocol: no beat counting, no identifier, no channel named.
//!
//! - [`axi`]: an AXI4 link, five channels behind two ends.
pub mod axi;
pub mod router;
