// SPDX-License-Identifier: Apache-2.0
//! The memory model's second port, for simulation only: the controller's
//! native interface is 256 bits wide, and a client that streams from
//! memory reads it at that width rather than a word at a time (issue
//! 1696).
//!
//! [`Words`] is the model's memory, which [`Ddr3`](super::Ddr3) keeps in
//! a field so that it can be handed to a [`WideRead`] beside it. Both
//! then read and write one memory, as two ports on one controller do.

use super::{AW, MODEL_READ_LATENCY};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use txhdl::comp::trace::{Scope, Traceable};
use txhdl::comp::{Clock, DefaultClock, Rx, Tx, Unit};
use txhdl::netlist::Port;
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{Ar, Resp, R};

/// The model's memory: its 32-bit words by index, the word at byte
/// address `a` being index `a / 4`, taken modulo the memory's size.
/// Cloning it gives another handle on the same words.
#[derive(Clone, Default)]
pub struct Words(pub Rc<RefCell<HashMap<u128, U<32>>>>);

impl Words {
    /// The word at index `i`, zero if nothing wrote it.
    pub fn word(&self, i: u128) -> U<32> {
        let i = i % (1u128 << AW);
        self.0.borrow().get(&i).copied().unwrap_or_default()
    }
}

/// The words are the model's state and not a signal: nothing to trace.
impl Traceable for Words {
    fn trace(&self, _: &Scope) {}
}

/// Nor a field of the netlist: the controller is a foreign module.
impl Port for Words {}

/// The bytes in one beat of the wide port.
pub const WIDE_BYTES: u128 = 32;

/// A read port 256 bits wide on the model's memory, standing in for the
/// controller's native interface in simulation. It takes AXI4 read
/// address phases and answers each with incrementing beats of 32 bytes
/// from the address rounded down to 32 bytes, `len + 1` of them, the
/// word at the lowest address in bits 31:0 of the first limb and the
/// eighth in bits 127:96 of the second. Its first beat is offered
/// [`MODEL_READ_LATENCY`] steps after the address was taken, as the
/// 32-bit port's is; after that one beat a step while the sink is ready,
/// and bursts in the order their addresses came. It takes the words when
/// it takes the address, as the 32-bit port does (issue 1424), and is
/// never anything but `OKAY`. `AxSIZE` and `AxBURST` are not read.
#[derive(Default)]
pub struct WideRead<const I: usize> {
    /// The memory it reads.
    pub words: Words,
}

impl<const I: usize> WideRead<I> {
    /// A port reading `words`.
    pub fn on(words: Words) -> Self {
        WideRead { words }
    }

    /// The beat of 32 bytes at word index `first`, as the port sends it.
    pub fn beat(&self, first: u128) -> U<256, 2> {
        let mut limbs = [0u128; 2];
        for j in 0..8 {
            let w = self.words.word(first + j as u128).raw();
            limbs[j / 4] |= w << (32 * (j % 4));
        }
        U::from_limbs(limbs)
    }
}

impl<const I: usize> Traceable for WideRead<I> {
    fn trace(&self, _: &Scope) {}
}

impl<const I: usize> Unit<Rx<Ar<32, I>>, Tx<R<256, I, 2>>> for WideRead<I> {
    async fn run(&mut self, ar: Rx<Ar<32, I>>, r: Tx<R<256, I, 2>>) {
        // Beats taken and not yet sent, each with the step it may be
        // offered from.
        let mut q: VecDeque<(R<256, I, 2>, u64)> = VecDeque::new();
        let mut now: u64 = 0;
        loop {
            DefaultClock::rising().await;
            now += 1;
            if let Some(a) = ar.recv() {
                let first = (a.addr.raw() & !(WIDE_BYTES - 1)) >> 2;
                let n = a.len.raw() + 1;
                let due = now + MODEL_READ_LATENCY as u64;
                for k in 0..n {
                    let beat = R {
                        id: a.id,
                        data: self.beat(first + 8 * k),
                        resp: Resp::Okay,
                        last: Bit::from_bool(k + 1 == n),
                    };
                    q.push_back((beat, due));
                }
            }
            if matches!(q.front(), Some(&(_, due)) if due <= now)
                && r.ready().to_bool()
            {
                r.send(q.pop_front().unwrap().0);
            }
        }
    }
}
