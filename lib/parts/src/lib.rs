// SPDX-License-Identifier: Apache-2.0
//! Parts: what the language provides as hardware, written in its own
//! lowerable subset and checked as every unit is. A unit's channel
//! port is one side of a channel; the channel itself, the elastic
//! buffer of two between two units, is the part here.
pub mod buffer;
