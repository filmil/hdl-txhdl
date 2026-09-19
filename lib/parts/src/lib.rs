// SPDX-License-Identifier: Apache-2.0
//! Parts: what the language provides as hardware, written in its own
//! lowerable subset and checked as every unit is. A unit's channel
//! port is one side of a channel; the channel itself, the elastic
//! buffer of two between two units, is the first part here; a FIFO
//! with a channel at each end and a reservation station are the
//! others.

// As in the runtime crate: every public item is documented and the
// build refuses one that is not. A part written by a macro is
// documented by its generator, so the text arrives with the code.
#![deny(missing_docs)]

pub mod buffer;
pub mod bus;
pub mod eth;
pub mod fifo;
pub mod flashwin;
pub mod gpio;
pub mod hdmi;
pub mod i2c;
pub mod plic;
pub mod pwm;
pub mod redundant;
pub mod spi;
pub mod station;
pub mod syscon;
pub mod tracer;
pub mod wdog;
