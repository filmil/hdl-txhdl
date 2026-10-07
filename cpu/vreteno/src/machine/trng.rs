// SPDX-License-Identifier: Apache-2.0
//! The entropy source as the machine model has it (issue 1386): the
//! registers of `txhdl_parts::trng` at the part's own offsets, read by
//! name from its map, and in place of the rings a deterministic
//! sequence, so that a run is the same every time.
//!
//! Zephyr's driver (`zephyr/drivers/entropy/entropy_vreteno.c`) sets
//! the run bit, and for each word reads the status, gives up on a
//! fault, waits for a word to be ready, and reads it. Here a word is
//! ready whenever the run bit is set, the faults never trip, and the
//! capture, which is for looking at the rings' samples, reads empty.
//! Without the source a Zephyr image that asks for entropy, as the
//! fastboot server's network stack does, faulted at its first read.
use txhdl_parts::trng::regs;

/// The source's registers, and the sequence's state.
#[derive(Debug)]
pub struct Trng {
    /// The run bit.
    pub run: bool,
    /// An xorshift's state: the words the source gives, in order.
    pub state: u64,
    /// Words given since the reset.
    pub words: u64,
}

impl Default for Trng {
    fn default() -> Self {
        Trng {
            run: false,
            state: 0x9e37_79b9_7f4a_7c15,
            words: 0,
        }
    }
}

impl Trng {
    /// The sequence's next word.
    fn next(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        self.words += 1;
        (x >> 32) as u32
    }

    /// A word read at byte offset `off`. Reading the data word takes
    /// it, as on the part, so the read changes the state.
    pub fn load(&mut self, off: u32) -> u32 {
        match off {
            regs::data if self.run => self.next(),
            // Ready, a full buffer of seven, and the run bit read back,
            // while it runs; nothing otherwise. The faults never trip.
            regs::status if self.run => 1 | (7 << 1) | (1 << 9),
            regs::ctrl => self.run as u32,
            regs::raw if self.run => self.next(),
            _ => 0,
        }
    }

    /// A word written at byte offset `off`: the run bit. The clear
    /// clears faults this model never has.
    pub fn store(&mut self, off: u32, v: u32) {
        if off == regs::ctrl {
            self.run = v & 1 != 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing is ready until it runs; then every read of the data
    /// word is a new word, and the same sequence every time.
    #[test]
    fn the_source_gives_words_while_it_runs() {
        let mut t = Trng::default();
        assert_eq!(t.load(regs::status) & 1, 0, "not ready stopped");
        assert_eq!(t.load(regs::data), 0);
        t.store(regs::ctrl, 0b11);
        assert_eq!(t.load(regs::ctrl), 1, "the run bit reads back");
        let s = t.load(regs::status);
        assert_eq!(s & 1, 1, "ready");
        assert_eq!(s >> 9 & 1, 1, "running");
        assert_eq!(s & (1 << 8 | 1 << 10), 0, "no fault");
        let (a, b) = (t.load(regs::data), t.load(regs::data));
        assert_ne!(a, b, "a word each read");
        let mut u = Trng::default();
        u.store(regs::ctrl, 1);
        assert_eq!(u.load(regs::data), a, "the same sequence each run");
    }
}
