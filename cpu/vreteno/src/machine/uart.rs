// SPDX-License-Identifier: Apache-2.0
//! The serial port as SiFive's `sifive,uart0`, the map #1011 gives the
//! hardware and Linux's, OpenSBI's and U-Boot's drivers expect
//! (issue 1016).
//!
//! Outside the timing mode the model sends a byte the moment it is
//! written, so the transmit queue is always empty, `txdata` never reads
//! full, and `txdrop`, the bytes dropped from a full queue (issue 1779),
//! stays zero. In the timing mode the port is paced as the hardware's is
//! (issue 1561): a byte joins a queue of eight, the oldest goes out when
//! the line is free, and each takes ten bit times, or eleven with
//! `nstop`, of `div` plus one cycles. `txdata` reads full while eight
//! wait, and a byte written then is dropped and counted in `txdrop` as
//! the write lands, which is when the hardware counts it. Received
//! bytes wait in a queue the machine fills, from a file or a test.
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
pub const TXDROP: u32 = 0x1c;

/// `rxdata`'s empty bit, and `txdata`'s full bit.
const EMPTY: u32 = 1 << 31;

/// The transmit queue's depth, the hardware's.
pub const TX_DEPTH: usize = 8;

/// The port: its control words, what it has sent, and what waits to be
/// read.
#[derive(Clone, Debug)]
pub struct Uart {
    pub txctrl: u32,
    pub rxctrl: u32,
    pub ie: u32,
    pub div: u32,
    /// Every byte sent, in order: in the timing mode, as it starts out
    /// on the line.
    pub sent: Vec<u8>,
    /// Bytes received and not yet read.
    pub rx: VecDeque<u8>,
    /// Whether bytes go out at the line's rate, the timing mode's
    /// setting, which the machine gives the port each step.
    pub paced: bool,
    /// Bytes written and waiting for the line, at most [`TX_DEPTH`].
    pub tx: VecDeque<u8>,
    /// When the line is next free, in the machine's clock.
    free_at: u64,
    /// The machine's clock as last given.
    now: u64,
    /// `txdrop`: bytes written to a full queue, wrapping at 16 bits.
    pub txdrop: u16,
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
            paced: false,
            tx: VecDeque::new(),
            free_at: 0,
            now: 0,
            txdrop: 0,
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
        let txwm = ((self.tx.len() as u32) < self.tx_mark()) as u32;
        let rxwm = (self.rx.len() > self.rx_mark()) as u32;
        txwm | rxwm << 1
    }

    /// The cycles a byte takes on the line: a start bit, eight data bits
    /// and one stop bit, or two with `nstop`, each `div` plus one long.
    pub fn byte_time(&self) -> u64 {
        let bits = 10 + serial::txctrl_nstop.get(self.txctrl) as u64;
        bits * (self.div as u64 + 1)
    }

    /// The port at `now` in the machine's clock: in the timing mode, each
    /// waiting byte whose turn on the line has come goes out. Whether the
    /// queue's length changed, which moves the transmit watermark.
    pub fn at(&mut self, now: u64) -> bool {
        self.now = now;
        if !self.paced || self.tx.is_empty() {
            return false;
        }
        let txen = self.txen();
        let was = self.tx.len();
        // The queue has not been empty since the line last freed, so each
        // byte starts as the one before it ends.
        while txen && self.free_at <= now {
            let Some(b) = self.tx.pop_front() else { break };
            self.sent.push(b);
            self.free_at += self.byte_time();
        }
        self.tx.len() != was
    }

    /// Everything the program wrote that the line sends: what has gone
    /// out and, while `txen` is set, what still waits, which the board's
    /// port goes on sending after its hart stops.
    pub fn written(&self) -> Vec<u8> {
        let mut all = self.sent.clone();
        if self.txen() {
            all.extend(self.tx.iter());
        }
        all
    }

    /// What still waits sent at once, as the line sends it once the
    /// machine has stopped running the hart; nothing while `txen` is
    /// clear, since the port holds it then.
    pub fn drain(&mut self) {
        if self.txen() {
            let waiting = std::mem::take(&mut self.tx);
            self.sent.extend(waiting);
        }
    }

    fn txen(&self) -> bool {
        serial::txctrl_txen.get(self.txctrl) == 1
    }

    /// A word read at `off`. Reading `rxdata` takes a byte.
    pub fn load(&mut self, off: u32) -> u32 {
        match off {
            TXDATA => {
                if self.tx.len() == TX_DEPTH {
                    EMPTY
                } else {
                    0
                }
            }
            RXDATA => match self.rx.pop_front() {
                Some(b) => b as u32,
                None => EMPTY,
            },
            TXCTRL => self.txctrl,
            RXCTRL => self.rxctrl,
            IE => self.ie,
            IP => self.ip(),
            DIV => self.div,
            TXDROP => self.txdrop as u32,
            _ => 0,
        }
    }

    /// A word written at `off`. Writing `txdata` sends its low byte: at
    /// once, or in the timing mode into the queue, or into nothing when
    /// eight wait already.
    pub fn store(&mut self, off: u32, v: u32) {
        match off {
            TXDATA if !self.paced => self.sent.push(v as u8),
            TXDATA if self.tx.len() == TX_DEPTH => {
                self.txdrop = self.txdrop.wrapping_add(1);
            }
            TXDATA => {
                // An idle line starts the byte now, not when it last freed.
                if self.tx.is_empty() && self.free_at < self.now {
                    self.free_at = self.now;
                }
                self.tx.push_back(v as u8);
                self.at(self.now);
            }
            TXCTRL => {
                // A line held by `txen` clear starts again from now.
                if self.free_at < self.now {
                    self.free_at = self.now;
                }
                self.txctrl = v;
                self.at(self.now);
            }
            RXCTRL => self.rxctrl = v,
            IE => self.ie = v & 3,
            // The divider is sixteen bits, as the hardware's is.
            DIV => self.div = v & 0xffff,
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
        assert_eq!(u.load(TXDROP), 0, "none dropped");
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

    /// In the timing mode a byte takes its ten bit times on the line,
    /// eight wait behind it, `txdata` reads full while they do, and a
    /// ninth written then is dropped and counted (issue 1561).
    #[test]
    fn paced_bytes_leave_at_the_line_s_rate() {
        let mut u = Uart {
            paced: true,
            ..Uart::default()
        };
        u.store(DIV, 867);
        let t = u.byte_time();
        assert_eq!(t, 8_680, "115200 baud at 100 MHz");
        u.at(1_000);
        for b in 0..10u8 {
            u.store(TXDATA, b as u32);
        }
        assert_eq!(u.sent, [0], "one on the line at once");
        assert_eq!(u.tx.len(), TX_DEPTH, "eight waiting");
        assert_eq!(u.load(TXDATA) & EMPTY, EMPTY, "full");
        assert_eq!(u.load(TXDROP), 1, "the tenth dropped as written");
        assert!(!u.at(1_000 + t - 1), "the first still going out");
        assert!(u.at(1_000 + t), "the next starts as the first ends");
        assert_eq!(u.sent, [0, 1]);
        assert_eq!(u.load(TXDATA) & EMPTY, 0, "room for one");
        // Back to back: the eighth waiting goes out eight byte times on.
        u.at(1_000 + 9 * t);
        assert_eq!(
            u.sent,
            (0..9).collect::<Vec<u8>>(),
            "the dropped one never"
        );
        // The transmit watermark rises as the queue empties.
        u.store(
            TXCTRL,
            serial::txctrl_txen.with(1) | serial::txctrl_txcnt.with(1),
        );
        assert_eq!(u.load(IP) & 1, 1, "fewer than one waiting");
        // An idle line starts a byte when it is written, not when it
        // last freed; two stop bits make it eleven bit times.
        u.store(
            TXCTRL,
            serial::txctrl_txen.with(1) | serial::txctrl_nstop.with(1),
        );
        assert_eq!(u.byte_time(), 9_548);
        u.at(1_000_000);
        u.store(TXDATA, 0x41);
        u.store(TXDATA, 0x42);
        assert!(!u.at(1_000_000 + 9_547));
        assert!(u.at(1_000_000 + 9_548));
        assert_eq!(&u.sent[9..], b"AB");
        // What waits is written all the same, and goes once drained.
        u.store(TXDATA, b'C' as u32);
        u.store(TXDATA, b'D' as u32);
        assert_eq!(&u.written()[9..], b"ABCD", "sent and waiting");
        u.drain();
        assert_eq!(&u.sent[9..], b"ABCD", "drained");
        assert!(u.tx.is_empty());
    }

    /// `txen` cleared with a byte on the line: the byte finishes, as the
    /// RTL's shifter does whatever `txen` says, the next waits, and is
    /// neither written nor drained while it waits; set again, the next
    /// starts then, or when the line frees if that is later. The
    /// transmit watermark rises as the queue empties.
    #[test]
    fn txen_cleared_mid_byte_holds_the_next() {
        let mut u = Uart {
            paced: true,
            ..Uart::default()
        };
        u.store(DIV, 9); // ten cycles a bit, a hundred a byte
        let on = serial::txctrl_txen.with(1) | serial::txctrl_txcnt.with(1);
        u.store(TXCTRL, on);
        u.store(TXDATA, b'a' as u32);
        u.store(TXDATA, b'b' as u32);
        assert_eq!(u.load(IP) & 1, 0, "one waits: not below a mark of 1");
        u.at(50);
        u.store(TXCTRL, serial::txctrl_txcnt.with(1));
        u.at(150);
        assert_eq!(u.sent, b"a", "a finished, b held");
        assert_eq!(u.written(), b"a", "nothing held is written");
        u.drain();
        assert_eq!(u.tx.len(), 1, "nor drained");
        u.at(300);
        u.store(TXCTRL, on);
        assert_eq!(u.sent, b"ab", "b starts as txen is set");
        assert_eq!(u.load(IP) & 1, 1, "the queue empty: below the mark");
        // Set again while a byte is still on the line, the next waits for
        // the line.
        u.store(TXDATA, b'c' as u32);
        u.store(TXDATA, b'd' as u32);
        u.store(TXCTRL, serial::txctrl_txcnt.with(1));
        u.at(350);
        u.store(TXCTRL, on);
        assert_eq!(u.sent, b"ab", "c waits for b to finish at 400");
        u.at(400);
        assert_eq!(u.sent, b"abc", "c as the line frees");
        u.at(499);
        assert_eq!(u.sent, b"abc");
        u.at(500);
        assert_eq!(u.sent, b"abcd", "d as the line frees");
        // The divider is sixteen bits.
        u.store(DIV, 0x1_0003);
        assert_eq!(u.load(DIV), 3);
    }

    /// A port held with `txen` clear keeps its queue, and sends it from
    /// when it is enabled again.
    #[test]
    fn a_paced_port_with_txen_clear_holds_its_queue() {
        let mut u = Uart {
            paced: true,
            ..Uart::default()
        };
        u.store(DIV, 3);
        u.store(TXCTRL, 0);
        u.store(TXDATA, 7);
        u.at(10_000);
        assert!(u.sent.is_empty(), "held");
        u.store(TXCTRL, serial::txctrl_txen.with(1));
        assert_eq!(u.sent, [7], "out as it is enabled");
    }

    /// The model's controls after reset are the hardware's, read from
    /// the registers `uart.rs` builds them in (issue 1097).
    #[test]
    fn the_controls_reset_as_the_hardware_s_do() {
        let hw = crate::uart::Uart::<868>::default();
        let bit = |b: txhdl::types::Bit| b.to_bool() as u32;
        let mut u = Uart::default();
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
        assert_eq!(u.load(TXDROP), hw.txdrop.get().raw() as u32, "txdrop");
        // Both directions enabled from reset, and `ie` clear, as SiFive's
        // and Linux's driver have it: a byte waiting raises nothing until
        // a program enables the watermark.
        assert_eq!(serial::txctrl_txen.get(u.txctrl), 1, "sending on");
        assert_eq!(serial::rxctrl_rxen.get(u.rxctrl), 1, "receiving on");
        u.rx.push_back(b'x');
        assert!(!u.irq(), "no interrupt before ie is written");
        u.store(IE, serial::ie_rxwm.with(1));
        assert!(u.irq(), "and one once it is");
    }
}
