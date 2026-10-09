// SPDX-License-Identifier: Apache-2.0
//! The core-local interruptor, as the machine model has it: `msip` at
//! 0, `mtimecmp` at `0x4000` and `mtime` at `0xbff8`, the offsets every
//! RISC-V platform uses and the board's timer decodes (issue 1016).
//!
//! `mtime` counts one a retired instruction, which is the timebase the
//! device tree states at the board's clock: the model has no cycles, and
//! an instruction a cycle is the core's best case. Software that waits
//! for a time therefore waits for that many instructions.
//!
//! Hart 1 has a software interrupt at `0x4` and a compare at `0x4008`,
//! and the mailbox's two words are at `0xc000`, as the board's timer
//! has them (issue 1408).

/// The CLINT's words, by offset.
pub const MSIP: u32 = 0x0;
/// The compare, low then high.
pub const MTIMECMP: u32 = 0x4000;
/// The count, low then high.
pub const MTIME: u32 = 0xbff8;
/// Hart 1's software interrupt and compare (issue 1408).
pub const MSIP1: u32 = 0x4;
pub const MTIMECMP1: u32 = 0x4008;
/// The mailbox: where hart 1 starts, and with what.
pub const MBOX_ENTRY: u32 = 0xc000;
pub const MBOX_ARG: u32 = 0xc004;

/// The CLINT: the software interrupt's bit, the compare and the count.
#[derive(Clone, Debug)]
pub struct Clint {
    pub msip: bool,
    pub mtimecmp: u64,
    pub mtime: u64,
    pub msip1: bool,
    pub mtimecmp1: u64,
    pub mbox_entry: u32,
    pub mbox_arg: u32,
}

impl Default for Clint {
    fn default() -> Self {
        Clint {
            msip: false,
            // The reset value is the largest, so no timer interrupt is
            // pending until software sets a compare.
            mtimecmp: u64::MAX,
            mtime: 0,
            msip1: false,
            mtimecmp1: u64::MAX,
            mbox_entry: 0,
            mbox_arg: 0,
        }
    }
}

impl Clint {
    /// A word read at `off`. Unnamed words read zero.
    pub fn load(&self, off: u32) -> u32 {
        match off {
            MSIP => self.msip as u32,
            o if o == MTIMECMP => self.mtimecmp as u32,
            o if o == MTIMECMP + 4 => (self.mtimecmp >> 32) as u32,
            o if o == MTIME => self.mtime as u32,
            o if o == MTIME + 4 => (self.mtime >> 32) as u32,
            MSIP1 => self.msip1 as u32,
            o if o == MTIMECMP1 => self.mtimecmp1 as u32,
            o if o == MTIMECMP1 + 4 => (self.mtimecmp1 >> 32) as u32,
            MBOX_ENTRY => self.mbox_entry,
            MBOX_ARG => self.mbox_arg,
            _ => 0,
        }
    }

    /// A word written at `off`. Unnamed words ignore it.
    pub fn store(&mut self, off: u32, v: u32) {
        let lo = |w: u64| (w & !0xffff_ffff) | u64::from(v);
        let hi = |w: u64| (w & 0xffff_ffff) | (u64::from(v) << 32);
        match off {
            MSIP => self.msip = v & 1 == 1,
            o if o == MTIMECMP => self.mtimecmp = lo(self.mtimecmp),
            o if o == MTIMECMP + 4 => self.mtimecmp = hi(self.mtimecmp),
            o if o == MTIME => self.mtime = lo(self.mtime),
            o if o == MTIME + 4 => self.mtime = hi(self.mtime),
            MSIP1 => self.msip1 = v & 1 == 1,
            o if o == MTIMECMP1 => self.mtimecmp1 = lo(self.mtimecmp1),
            o if o == MTIMECMP1 + 4 => self.mtimecmp1 = hi(self.mtimecmp1),
            MBOX_ENTRY => self.mbox_entry = v,
            MBOX_ARG => self.mbox_arg = v,
            _ => {}
        }
    }

    /// The count moves on by `n`.
    pub fn tick(&mut self, n: u64) {
        self.mtime = self.mtime.wrapping_add(n);
    }

    /// The timer's line: the count has reached the compare.
    pub fn mtip(&self) -> bool {
        self.mtime >= self.mtimecmp
    }

    /// Hart 1's timer line.
    pub fn mtip1(&self) -> bool {
        self.mtime >= self.mtimecmp1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timer_rises_when_the_count_reaches_the_compare() {
        let mut c = Clint::default();
        assert!(!c.mtip(), "no compare set, no interrupt");
        c.store(MTIMECMP + 4, 0);
        c.store(MTIMECMP, 10);
        assert_eq!(c.load(MTIMECMP), 10);
        c.tick(9);
        assert!(!c.mtip());
        c.tick(1);
        assert!(c.mtip(), "at the compare");
        assert_eq!(c.load(MTIME), 10);
        c.store(MSIP, 1);
        assert!(c.msip);
        assert_eq!(c.load(0x1234), 0, "an unnamed word reads zero");
    }
}
