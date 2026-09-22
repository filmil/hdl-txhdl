// SPDX-License-Identifier: Apache-2.0
//! A word to the remote peripheral and back, on the board.
//!
//! The peripheral at `0x3300` has no behaviour of its own: a store to
//! it leaves the board as an Ethernet frame, a program on another
//! machine answers, and a load reads back whatever that program says.
//! This program writes two words there, reads both back, says each
//! in hexadecimal on the serial port, and gives the verdict, which is
//! the last hop of issue 335 seen from the core: `remote ok` means a
//! frame left on the wire, was answered by a program on a laptop, and
//! the answer came back through the same port.
//!
//! Linked for the memory rather than the boot memory and sent down
//! the serial line by the loader, so the flagship bitstream does not
//! move. A peripheral that never answers is refused by the bus after
//! its patience runs out, and the read then returns what the bus
//! returns for a refusal, which the verdict calls bad.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use vreteno_hal::{entry, halt, map, Uart};

/// Two words at two addresses of the peripheral, so that the answer
/// is not one lucky value.
const WORDS: [(usize, u32); 2] = [(0x0, 0xc0ff_ee11), (0x4, 0x5eed_0335)];

entry!(main);

fn main() -> ! {
    let mut ok = true;
    let mut i = 0;
    while i < 2 {
        let (offset, word) = WORDS[i];
        let p = (map::REMOTE + offset) as *mut u32;
        Uart::say(b"remote: write ");
        Uart::put_hex(word);
        Uart::put(b'\n');
        unsafe { write_volatile(p, word) };
        let back = unsafe { read_volatile(p) };
        Uart::say(b"remote: read  ");
        Uart::put_hex(back);
        Uart::put(b'\n');
        ok = ok && back == word;
        i += 1;
    }
    Uart::say(if ok { b"remote ok\n" } else { b"remote bad\n" });
    halt()
}
