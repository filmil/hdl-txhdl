// SPDX-License-Identifier: Apache-2.0
//! The Ethernet PHY's management registers, read by the core over
//! MDIO (issue 864), printed on the serial port, and nothing written.
//!
//! * `phy at` and the first address, of the 32 MDIO allows, whose
//!   register 2 reads other than `ffff`. A line nobody drives reads
//!   all ones through its pull-up, so `ffff` is no PHY there. `no phy`
//!   if none answers.
//! * Eight lines, `r00` to `r28`, each the register named and the three
//!   after it, as four hexadecimal digits each. Registers 0 to 15 are
//!   the ones IEEE 802.3 clause 22 defines: 2 and 3 are the PHY's
//!   identifier. Registers 16 to 31 are the vendor's; which of them
//!   holds the JL2121's receive delay is in its data sheet, which could
//!   not be found, so this program prints them all rather than
//!   picking one.
//!
//! The program only reads. It never writes a register, not even a
//! page select, so it leaves the PHY as it found it. A read of
//! register 1 does clear its latched link-down bit, as clause 22
//! defines, which the PHY sets again if the link drops.
//!
//! MDC is 1.25 MHz from the board's 100 MHz, half the 2.5 MHz that
//! 802.3 allows, since the JL2121's own limit is not known.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use vreteno_hal::{entry, halt, map, Uart};
use vreteno_regs::mdio;

const MDIO: *mut u32 = map::MDIO as *mut u32;

/// Half of an MDC cycle is forty cycles: 1.25 MHz from 100 MHz.
const DIV: u32 = 39;

entry!(main);

/// One read frame, waited for, and the word it took.
fn read(phy: u32, reg: u32) -> u32 {
    let word = (phy << mdio::CMD_PHYAD_SHIFT) & mdio::CMD_PHYAD_MASK
        | (reg << mdio::CMD_REGAD_SHIFT) & mdio::CMD_REGAD_MASK;
    unsafe {
        write_volatile(MDIO.add(mdio::CMD / 4), word);
        while read_volatile(MDIO.add(mdio::STATE / 4)) & mdio::STATE_BUSY_MASK
            != 0
        {}
        read_volatile(MDIO.add(mdio::DATA / 4)) & 0xffff
    }
}

/// Four hexadecimal digits.
fn put_word(v: u32) {
    for i in (0..4).rev() {
        let d = (v >> (4 * i)) & 0xf;
        Uart::put(if d < 10 {
            b'0' + d as u8
        } else {
            b'a' + d as u8 - 10
        });
    }
}

/// Two decimal digits.
fn put_two(v: u32) {
    Uart::put(b'0' + (v / 10) as u8);
    Uart::put(b'0' + (v % 10) as u8);
}

fn main() -> ! {
    unsafe { write_volatile(MDIO.add(mdio::CTRL / 4), DIV) };
    let Some(phy) = (0..32).find(|&a| read(a, 2) != 0xffff) else {
        Uart::say(b"no phy\n");
        halt()
    };
    Uart::say(b"phy at ");
    Uart::put_decimal(phy);
    Uart::put(b'\n');
    for row in 0..8 {
        Uart::put(b'r');
        put_two(row * 4);
        for reg in row * 4..row * 4 + 4 {
            Uart::put(b' ');
            put_word(read(phy, reg));
        }
        Uart::put(b'\n');
    }
    halt()
}
