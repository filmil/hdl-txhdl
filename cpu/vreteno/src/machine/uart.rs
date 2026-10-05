// SPDX-License-Identifier: Apache-2.0
//! The serial port as SiFive's `sifive,uart0`, the map #1011 gives the
//! hardware and Linux's, OpenSBI's and U-Boot's drivers expect
//! (issue 1016).
//!
//! The model sends a byte the moment it is written, so the transmit
//! queue is always empty and `txdata` never reads full. Received bytes
//! wait in a queue the machine fills, from a file or a test.
//!
//! The controls reset to what the hardware's map declares, so a
//! program that never writes them finds the port running here as on
//! the board (issue 1097).
use crate::uart::serial;
use std::collections::VecDeque;

/// The words, by offset.
pub const TXDATA: u32 = 0x00;
pub const RXDATA: u32 = 0x04;
pub const TXCTRL: u32 = 0x08;
pub const RXCTRL: u32 = 0x0c;
pub const IE: u32 = 0x10;
pub const IP: u32 = 0x14;
pub const DIV: u32 = 0x18;

/// `rxdata`'s empty bit, and `txdata`'s full bit.
const EMPTY: u32 = 1 << 31;

/// The port: its control words, what it has sent, and what waits to be
/// read.
#[derive(Clone, Debug)]
pub struct Uart {
    pub txctrl: u32,
    pub rxctrl: u32,
    pub ie: u32,
    pub div: u32,
    /// Every byte sent, in order.
    pub sent: Vec<u8>,
    /// Bytes received and not yet read.
    pub rx: VecDeque<u8>,
}

/// The word the hardware's map declares register `name` resets to.
fn reset_of(name: &str) -> u32 {
    serial::MAP
        .regs
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("the serial map has no {name}"))
        .reset()
}

impl Default for Uart {
    /// The port after reset: the hardware's controls (issue 1097), both
    /// directions enabled, `ie` clear as SiFive's is, and the divider
    /// the map declares; nothing sent and nothing waiting.
    fn default() -> Self {
        Uart {
            txctrl: reset_of("txctrl"),
            rxctrl: reset_of("rxctrl"),
            ie: reset_of("ie"),
            div: reset_of("div"),
            sent: Vec::new(),
            rx: VecDeque::new(),
        }
    }
}

impl Uart {
    /// `rxctrl`'s watermark: an interrupt while more than this many
    /// bytes wait.
    fn rx_mark(&self) -> usize {
        ((self.rxctrl >> 16) & 7) as usize
    }

    /// `txctrl`'s watermark: an interrupt while fewer than this many
    /// bytes wait to go, which with an empty queue is any above zero.
    fn tx_mark(&self) -> u32 {
        (self.txctrl >> 16) & 7
    }

    /// `ip`: bit 0 the transmit watermark, bit 1 the receive one.
    fn ip(&self) -> u32 {
        let txwm = (self.tx_mark() > 0) as u32;
        let rxwm = (self.rx.len() > self.rx_mark()) as u32;
        txwm | rxwm << 1
    }

    /// A word read at `off`. Reading `rxdata` takes a byte.
    pub fn load(&mut self, off: u32) -> u32 {
        match off {
            TXDATA => 0,
            RXDATA => match self.rx.pop_front() {
                Some(b) => b as u32,
                None => EMPTY,
            },
            TXCTRL => self.txctrl,
            RXCTRL => self.rxctrl,
            IE => self.ie,
            IP => self.ip(),
            DIV => self.div,
            _ => 0,
        }
    }

    /// A word written at `off`. Writing `txdata` sends its low byte.
    pub fn store(&mut self, off: u32, v: u32) {
        match off {
            TXDATA => self.sent.push(v as u8),
            TXCTRL => self.txctrl = v,
            RXCTRL => self.rxctrl = v,
            IE => self.ie = v & 3,
            DIV => self.div = v,
            _ => {}
        }
    }

    /// The port's interrupt line: a watermark it is enabled for.
    pub fn irq(&self) -> bool {
        self.ip() & self.ie != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_go_out_and_come_in_with_the_bits_sifive_gives_them() {
        let mut u = Uart::default();
        assert_eq!(u.load(TXDATA) >> 31, 0, "never full");
        u.store(TXDATA, b'O' as u32);
        u.store(TXDATA, 0x100 | b'K' as u32);
        assert_eq!(u.sent, b"OK", "the low byte of each write");
        assert_eq!(u.load(RXDATA), EMPTY, "nothing received");
        u.rx.push_back(b'x');
        assert_eq!(u.load(RXDATA), b'x' as u32);
        assert_eq!(u.load(RXDATA) & EMPTY, EMPTY);
    }

    #[test]
    fn the_receive_watermark_raises_the_line_when_enabled() {
        let mut u = Uart::default();
        u.rx.push_back(1);
        assert!(!u.irq(), "not enabled");
        u.store(IE, 2);
        assert!(u.irq(), "a byte above a watermark of 0");
        u.store(RXCTRL, 1 << 16);
        assert!(!u.irq(), "one byte is not above a watermark of 1");
        u.store(TXCTRL, 1 << 16);
        u.store(IE, 1);
        assert!(u.irq(), "the empty transmit queue is below its mark");
    }

    /// The model's controls after reset are the hardware's, read from
    /// the registers `uart.rs` builds them in (issue 1097).
    #[test]
    fn the_controls_reset_as_the_hardware_s_do() {
        let hw = crate::uart::Uart::<868>::default();
        let bit = |b: txhdl::types::Bit| b.to_bool() as u32;
        let u = Uart::default();
        let txctrl = serial::txctrl_txen.with(bit(hw.txen.get()))
            | serial::txctrl_nstop.with(bit(hw.nstop.get()))
            | serial::txctrl_txcnt.with(hw.txcnt.get().raw() as u32);
        let rxctrl = serial::rxctrl_rxen.with(bit(hw.rxen.get()))
            | serial::rxctrl_rxcnt.with(hw.rxcnt.get().raw() as u32);
        let ie = serial::ie_txwm.with(bit(hw.ie_txwm.get()))
            | serial::ie_rxwm.with(bit(hw.ie_rxwm.get()));
        assert_eq!(u.txctrl, txctrl, "txctrl");
        assert_eq!(u.rxctrl, rxctrl, "rxctrl");
        assert_eq!(u.ie, ie, "ie");
        assert_eq!(u.div, hw.div.get().raw() as u32, "div");
        // Both directions enabled from reset, and `ie` clear, as SiFive's
        // and Linux's driver have it: a byte waiting raises nothing until
        // a program enables the watermark.
        assert_eq!(serial::txctrl_txen.get(u.txctrl), 1, "sending on");
        assert_eq!(serial::rxctrl_rxen.get(u.rxctrl), 1, "receiving on");
        let mut u = u;
        u.rx.push_back(b'x');
        assert!(!u.irq(), "no interrupt before ie is written");
        u.store(IE, serial::ie_rxwm.with(1));
        assert!(u.irq(), "and one once it is");
    }
}
