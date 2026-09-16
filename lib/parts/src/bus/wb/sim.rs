// SPDX-License-Identifier: Apache-2.0
//! Simulation-only Wishbone peripherals, as `bus::axi::sim` holds the
//! AXI ones: what a test or a model puts behind a Wishbone host when
//! the point is the host. Nothing here lowers.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use txhdl::comp::{Clock, DefaultClock, In, Out, Unit};
use txhdl::types::{Bit, U};

// begin{mem}
/// A memory behind a pipelined Wishbone interface: words of 32 bits
/// by word address, as many as the address can name, held sparsely so
/// that a memory of gigabytes costs only the words written.
///
/// It answers one request at a time. It stalls for its first
/// `warmup` cycles, as a controller does while it calibrates. It takes
/// a request on a cycle with `cyc`, `stb` and no stall, stalls while
/// it works for `latency` cycles, which is at least one, and then
/// acknowledges for one cycle with the word read, having written the
/// lanes `sel` names if it was a write.
///
/// It is a handle on the words, so a test keeps one to look at.
pub struct WbMem<const AW: usize> {
    words: Rc<RefCell<HashMap<u128, u32>>>,
    latency: u32,
    warmup: u32,
}
// end{mem}

impl<const AW: usize> Clone for WbMem<AW> {
    fn clone(&self) -> Self {
        WbMem {
            words: self.words.clone(),
            latency: self.latency,
            warmup: self.warmup,
        }
    }
}

impl<const AW: usize> WbMem<AW> {
    /// A memory whose every word is zero, answering after `latency`
    /// cycles and stalling for its first `warmup`.
    pub fn new(latency: u32, warmup: u32) -> Self {
        assert!(latency >= 1, "a request takes at least a cycle");
        WbMem {
            words: Rc::new(RefCell::new(HashMap::new())),
            latency,
            warmup,
        }
    }

    /// The word at a word address.
    pub fn word(&self, at: u128) -> u32 {
        *self.words.borrow().get(&at).unwrap_or(&0)
    }

    /// Set the word at a word address.
    pub fn set(&self, at: u128, v: u32) {
        self.words.borrow_mut().insert(at, v);
    }
}

/// The Wishbone host's lines, as the memory reads them: `cyc`, `stb`,
/// `we`, the word address, the word written and the lanes it writes.
pub type WbHostLines<const AW: usize> =
    (In<Bit>, In<Bit>, In<Bit>, In<U<AW>>, In<U<32>>, In<U<4>>);

/// The memory's lines, as the host reads them: `stall`, `ack` and the
/// word read.
pub type WbPerLines = (Out<Bit>, Out<Bit>, Out<U<32>>);

impl<const AW: usize> Unit<WbHostLines<AW>, WbPerLines> for WbMem<AW> {
    async fn run(
        &mut self,
        (cyc, stb, we, adr, dat, sel): WbHostLines<AW>,
        (stall, ack, rdat): WbPerLines,
    ) {
        // The request in hand, the cycles left on it, and what is put
        // out: every output a register, as a controller's are.
        let mut left = 0u32;
        let mut warm = self.warmup;
        let mut req = (false, 0u128, 0u32, 0u32);
        let mut acking = false;
        let mut word = 0u32;
        loop {
            DefaultClock::rising().await;
            stall.set(Bit::from_bool(left > 0 || warm > 0));
            ack.set(Bit::from_bool(acking));
            rdat.set(U::<32>::from(word));
            acking = false;
            if warm > 0 {
                warm -= 1;
            } else if left > 0 {
                left -= 1;
                if left == 0 {
                    let (write, at, value, lanes) = req;
                    let mut words = self.words.borrow_mut();
                    let old = *words.get(&at).unwrap_or(&0);
                    if write {
                        let mask = (0..4)
                            .filter(|l| lanes >> l & 1 == 1)
                            .fold(0u32, |m, l| m | 0xff << (8 * l));
                        words.insert(at, (old & !mask) | (value & mask));
                    }
                    word = if write { 0 } else { old };
                    acking = true;
                }
            } else if cyc.get().to_bool() && stb.get().to_bool() {
                req = (
                    we.get().to_bool(),
                    adr.get().raw(),
                    dat.get().raw() as u32,
                    sel.get().raw() as u32,
                );
                left = self.latency;
            }
        }
    }
}
