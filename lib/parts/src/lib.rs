// SPDX-License-Identifier: Apache-2.0
//! Parts: what the language provides as hardware, written in its own
//! lowerable subset and checked as every unit is. A unit's channel
//! port is one side of a channel; the channel itself, the elastic
//! buffer of two between two units, is the first part here; a FIFO
//! with a channel at each end and a reservation station are the
//! others.
pub mod buffer;
pub mod fifo;
pub mod station;
