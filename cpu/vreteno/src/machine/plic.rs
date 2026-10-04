// SPDX-License-Identifier: Apache-2.0
//! The platform-level interrupt controller, as the machine model has it:
//! the part in `lib/parts/src/plic.rs` with its two targets, machine and
//! supervisor, at the same offsets (issues 1013 and 1016).
//!
//! Every source here asks while its line is high, which is what the
//! board's three are. A source's request goes forward only when it is
//! not in service, from the claim that took it to the complete that ends
//! it, by either target.

/// The words, by offset.
pub const PENDING: u32 = 0x1000;
/// Each target's enables, thresholds and claim words, machine first.
pub const ENABLE: [u32; 2] = [0x2000, 0x2080];
pub const THRESHOLD: [u32; 2] = [0x20_0000, 0x20_1000];
pub const CLAIM: [u32; 2] = [0x20_0004, 0x20_1004];

/// The controller of `n` sources, numbered from 1.
#[derive(Clone, Debug)]
pub struct Plic {
    n: usize,
    /// Each source's priority, `prio[s]` for source `s`; `prio[0]`
    /// unused.
    pub prio: Vec<u32>,
    /// Requests not yet claimed, bit `s` for source `s`.
    pub pending: u32,
    /// Requests claimed and not yet completed.
    pub active: u32,
    /// The lines.
    pub lines: u32,
    pub enable: [u32; 2],
    pub threshold: [u32; 2],
}

impl Plic {
    pub fn new(n: usize) -> Self {
        assert!((1..=31).contains(&n), "one to thirty-one sources");
        Plic {
            n,
            prio: vec![0; n + 1],
            pending: 0,
            active: 0,
            lines: 0,
            enable: [0; 2],
            threshold: [0; 2],
        }
    }

    /// The bits that are sources: 1 to `n`.
    fn sources(&self) -> u32 {
        (((1u64 << (self.n + 1)) - 1) as u32) & !1
    }

    /// Source `s`'s line.
    pub fn line(&mut self, s: usize, high: bool) {
        if high {
            self.lines |= 1 << s;
        } else {
            self.lines &= !(1 << s);
        }
        self.forward();
    }

    /// A level source asks while its line is high, when not in service.
    fn forward(&mut self) {
        self.pending |= self.lines & !self.active & self.sources();
    }

    /// The source a claim by target `t` would take: pending, enabled,
    /// above the threshold, the highest priority, the lowest number on
    /// a tie. Zero for none.
    fn best(&self, t: usize) -> u32 {
        let mut best = 0;
        let mut best_pr = 0;
        for s in 1..=self.n {
            let p = self.prio[s];
            let cand = self.pending & self.enable[t] & (1 << s) != 0
                && p > self.threshold[t];
            if cand && p > best_pr {
                best = s as u32;
                best_pr = p;
            }
        }
        best
    }

    /// Target `t`'s line: a claim would take a source.
    pub fn irq(&self, t: usize) -> bool {
        self.best(t) != 0
    }

    /// A word read at `off`. A read of a claim word takes the request.
    pub fn load(&mut self, off: u32) -> u32 {
        for t in 0..2 {
            if off == CLAIM[t] {
                let s = self.best(t);
                if s != 0 {
                    self.pending &= !(1 << s);
                    self.active |= 1 << s;
                }
                return s;
            }
            if off == ENABLE[t] {
                return self.enable[t];
            }
            if off == THRESHOLD[t] {
                return self.threshold[t];
            }
        }
        if off == PENDING {
            return self.pending;
        }
        let s = (off / 4) as usize;
        if off.is_multiple_of(4) && (1..=self.n).contains(&s) {
            return self.prio[s];
        }
        0
    }

    /// A word written at `off`. A write of a source's number to a claim
    /// word completes it.
    pub fn store(&mut self, off: u32, v: u32) {
        for t in 0..2 {
            if off == CLAIM[t] {
                if (v as usize) <= self.n {
                    self.active &= !(1 << v);
                    self.forward();
                }
                return;
            }
            if off == ENABLE[t] {
                self.enable[t] = v & self.sources();
                return;
            }
            if off == THRESHOLD[t] {
                self.threshold[t] = v & 7;
                return;
            }
        }
        let s = (off / 4) as usize;
        if off.is_multiple_of(4) && (1..=self.n).contains(&s) {
            self.prio[s] = v & 7;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same rules as the part's own tests: the priority decides, a
    /// tie goes to the lower number, and a claimed source asks again
    /// only after its complete.
    #[test]
    fn claims_follow_the_parts_rules_for_either_target() {
        let mut p = Plic::new(3);
        p.store(4, 1);
        p.store(8, 3);
        p.store(12, 3);
        p.store(ENABLE[0], 0xe);
        for s in 1..=3 {
            p.line(s, true);
        }
        assert!(p.irq(0) && !p.irq(1), "only the machine target enables");
        let order: Vec<u32> = (0..4).map(|_| p.load(CLAIM[0])).collect();
        assert_eq!(order, [2, 3, 1, 0]);
        p.line(3, false);
        for s in 1..=3 {
            p.store(CLAIM[0], s);
        }
        assert_eq!(p.load(PENDING), 0x6, "sources 1 and 2 still high");
        // The supervisor target on its own.
        p.store(ENABLE[0], 0);
        p.store(ENABLE[1], 0x2);
        assert!(p.irq(1) && !p.irq(0));
        assert_eq!(p.load(CLAIM[1]), 1);
        assert_eq!(p.load(CLAIM[0]), 0, "the machine target takes none");
    }
}
