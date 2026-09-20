// SPDX-License-Identifier: Apache-2.0
//! A program for Vreteno written in Rust: it says a line on the serial
//! port and stops.
//!
//! What a program needs from the machine comes from `vreteno_hal`: the
//! serial port as a type, the reset path from `entry!`, and `halt` to
//! say it is done. This file is the program and nothing else, which is
//! the point of the crate (issue 142); what the crate holds, a naked
//! `_start`, addresses copied from the map, a panic handler, this file
//! used to hold too, and every program after it would have again.
//!
//! Nothing here indexes a slice with a variable and nothing divides,
//! so nothing asks for `core`'s panic path, which is larger than the
//! whole boot memory.
#![no_std]
#![no_main]

use vreteno_hal::{entry, halt, Uart};

/// What the program prints. A `static` rather than a literal in a
/// function, to say plainly that it is data and lives in the boot
/// memory beside the code, where a load reaches it over the bus; a
/// literal would too, but this is the thing being shown.
static GREETING: &[u8] = b"hello from rust\n";

entry!(main);

fn main() -> ! {
    Uart::say(GREETING);
    halt()
}
