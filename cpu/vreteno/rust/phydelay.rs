// SPDX-License-Identifier: Apache-2.0
//! The JL2121's RGMII delay bits, read over MDIO from register 17 of
//! page 3336, where JLSemi's Linux driver keeps them (issue 869).
//!
//! Selecting a page means writing register 31, so this program writes
//! the PHY, and twice: the page to read, and then the page it found.
//! It does what the driver's `jlsemi_read_paged` does and nothing more:
//!
//! 1. read register 31, the page select, and keep it;
//! 2. write 3336 to register 31;
//! 3. read register 17;
//! 4. write the kept page back to register 31;
//! 5. read register 31 again, to show the page was restored.
//!
//! Those two writes are the only writes. MDIO reports no errors, so a
//! read fails the one way it can: all ones, which is a line nobody
//! drives. If no PHY answers, or the first read of register 31 comes
//! back `ffff`, the program says so and writes nothing.
//!
//! It prints, one to a line: `phy at` and the address, `page was` and
//! register 31 as found, `p3336 r17` and the word, `tx delay` and bit 8,
//! `rx delay` and bit 9, and `page now` and register 31 as left; and
//! `page not restored` if the last two differ. Each delay bit is 2 ns
//! of delay on that clock when set, by the driver's
//! `JL2XXX_RGMII_TX_DLY_2NS` and `JL2XXX_RGMII_RX_DLY_2NS`.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use vreteno_hal::{entry, halt, map, Uart};
use vreteno_regs::mdio;

const MDIO: *mut u32 = map::MDIO as *mut u32;

/// Half of an MDC cycle is forty cycles: 1.25 MHz from 100 MHz, as
/// `phyregs` runs it.
const DIV: u32 = 39;

/// The page select, and the page and register of the delay bits.
const PAGE_SELECT: u32 = 31;
const RGMII_PAGE: u32 = 3336;
const RGMII_CTRL: u32 = 17;

entry!(main);

fn cmd(phy: u32, reg: u32) -> u32 {
    (phy << mdio::CMD_PHYAD_SHIFT) & mdio::CMD_PHYAD_MASK
        | (reg << mdio::CMD_REGAD_SHIFT) & mdio::CMD_REGAD_MASK
}

/// One frame, waited for.
fn frame(word: u32) {
    unsafe {
        write_volatile(MDIO.add(mdio::CMD / 4), word);
        while read_volatile(MDIO.add(mdio::STATE / 4)) & mdio::STATE_BUSY_MASK
            != 0
        {}
    }
}

/// One read frame, and the word it took.
fn read(phy: u32, reg: u32) -> u32 {
    frame(cmd(phy, reg));
    unsafe { read_volatile(MDIO.add(mdio::DATA / 4)) & 0xffff }
}

/// One write frame. The only caller writes register 31.
fn write(phy: u32, reg: u32, word: u32) {
    frame(
        cmd(phy, reg)
            | mdio::CMD_WRITE_MASK
            | (word << mdio::CMD_WORD_SHIFT) & mdio::CMD_WORD_MASK,
    );
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

fn line(label: &[u8], word: u32) {
    Uart::say(label);
    put_word(word);
    Uart::put(b'\n');
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
    let was = read(phy, PAGE_SELECT);
    if was == 0xffff {
        Uart::say(b"page read failed, nothing written\n");
        halt()
    }
    line(b"page was ", was);
    write(phy, PAGE_SELECT, RGMII_PAGE);
    let ctrl = read(phy, RGMII_CTRL);
    write(phy, PAGE_SELECT, was);
    let now = read(phy, PAGE_SELECT);
    line(b"p3336 r17 ", ctrl);
    Uart::say(b"tx delay ");
    Uart::put_decimal((ctrl >> 8) & 1);
    Uart::say(b"\nrx delay ");
    Uart::put_decimal((ctrl >> 9) & 1);
    Uart::put(b'\n');
    line(b"page now ", now);
    if now != was {
        Uart::say(b"page not restored\n");
    }
    halt()
}
