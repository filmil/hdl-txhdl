// SPDX-License-Identifier: Apache-2.0
//! A program that prints without end: a count and a newline, again and
//! again, so a test can see when the serial port stops and starts
//! (issue 1412).
#![no_std]
#![no_main]

use vreteno_hal::{entry, Uart};

entry!(main);

fn main() -> ! {
    let mut n = 0u32;
    loop {
        Uart::put_decimal(n);
        Uart::put(b'\n');
        n = n.wrapping_add(1);
    }
}
