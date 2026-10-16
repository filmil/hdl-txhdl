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
use txhdl::comp::{Clock, DefaultClock, In, Out, Rx, Tx, Unit};
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

/// The wide port's pins as the model reads them: `ARID`, `ARADDR`,
/// `ARLEN`, `ARVALID` and `RREADY` (issue 1696).
pub type WidePinsIn = (In<U<5>>, In<U<32>>, In<U<8>>, In<Bit>, In<Bit>);

/// The wide port's pins as the model drives them: `ARREADY`, `RID`,
/// `RDATA`, `RRESP`, `RLAST` and `RVALID`.
pub type WidePinsOut = (
    Out<Bit>,
    Out<U<5>>,
    Out<U<256, 2>>,
    Out<U<2>>,
    Out<Bit>,
    Out<Bit>,
);

/// Answer the wide port's reads on its pins for ever, as [`WideRead`]
/// answers them on a channel: beats of 32 bytes from the address
/// rounded down to 32 bytes, the first [`MODEL_READ_LATENCY`] steps after
/// the address was taken, then one a step while `RREADY` is high, and
/// not ready for the first `warmup` steps. The pins are read as the unit
/// drove them a step before, as `PinRam` reads its own (issue 1696).
pub async fn serve_wide_pins(
    words: Words,
    (arid, araddr, arlen, arvalid, rready): WidePinsIn,
    (arready, rid, rdata, rresp, rlast, rvalid): WidePinsOut,
    warmup: u64,
) {
    let port = WideRead::<5>::on(words);
    let mut q: VecDeque<(U<5>, U<256, 2>, bool, u64)> = VecDeque::new();
    let (mut ready, mut offered) = (false, false);
    let mut now: u64 = 0;
    loop {
        DefaultClock::rising().await;
        now += 1;
        if ready && arvalid.get().to_bool() {
            let first = (araddr.get().raw() & !(WIDE_BYTES - 1)) >> 2;
            let n = arlen.get().raw() + 1;
            let due = now + MODEL_READ_LATENCY as u64;
            for k in 0..n {
                let data = port.beat(first + 8 * k);
                q.push_back((arid.get(), data, k + 1 == n, due));
            }
        }
        if offered && rready.get().to_bool() {
            q.pop_front();
        }
        ready = now > warmup;
        arready.set(Bit::from_bool(ready));
        offered = matches!(q.front(), Some(&(_, _, _, due)) if due <= now);
        if let Some(&(id, data, last, _)) = q.front() {
            rid.set(id);
            rdata.set(data);
            rresp.set(U::<2>::from(0u8));
            rlast.set(Bit::from_bool(last));
        }
        rvalid.set(Bit::from_bool(offered));
    }
}

/// The wide port's pins, driven as a design would: two beats of eight
/// words each, from inside the first 32 bytes, at the model's latency
/// and then a beat a step.
#[cfg(test)]
mod tests {
    use super::{serve_wide_pins, Words, MODEL_READ_LATENCY};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use txhdl::comp::{join2, signal, Clock, DefaultClock, Running};
    use txhdl::types::{Bit, U};

    #[test]
    fn the_wide_pins_stream_a_burst() {
        let words = Words::default();
        for k in 0..16u128 {
            words
                .0
                .borrow_mut()
                .insert(0x400 + k, U::new(0x1000_0000 + k));
        }
        let (arid_o, arid) = signal::<U<5>, DefaultClock>();
        let (araddr_o, araddr) = signal::<U<32>, DefaultClock>();
        let (arlen_o, arlen) = signal::<U<8>, DefaultClock>();
        let (arvalid_o, arvalid) = signal::<Bit, DefaultClock>();
        let (rready_o, rready) = signal::<Bit, DefaultClock>();
        let (arready_o, arready) = signal::<Bit, DefaultClock>();
        let (rid_o, rid) = signal::<U<5>, DefaultClock>();
        let (rdata_o, rdata) = signal::<U<256, 2>, DefaultClock>();
        let (rresp_o, _rresp) = signal::<U<2>, DefaultClock>();
        let (rlast_o, rlast) = signal::<Bit, DefaultClock>();
        let (rvalid_o, rvalid) = signal::<Bit, DefaultClock>();
        let now = Rc::new(Cell::new(0u32));
        let got = Rc::new(RefCell::new(Vec::new()));
        let taken = Rc::new(Cell::new(None));
        let (at, into, took) = (now.clone(), got.clone(), taken.clone());
        // The design: offers one address once the port is ready, takes it
        // when ARREADY is high, and takes every beat.
        let design = async move {
            rready_o.set(Bit::One);
            let mut asked = false;
            loop {
                DefaultClock::rising().await;
                if rvalid.get().to_bool() {
                    into.borrow_mut().push((
                        at.get(),
                        rid.get().raw(),
                        rdata.get().limbs(),
                        rlast.get().to_bool(),
                    ));
                }
                if asked && arready.get().to_bool() {
                    arvalid_o.set(Bit::Zero);
                }
                if !asked && arready.get().to_bool() {
                    arid_o.set(U::new(7));
                    araddr_o.set(U::new(0x4000_1004));
                    arlen_o.set(U::new(1));
                    arvalid_o.set(Bit::One);
                    took.set(Some(at.get()));
                    asked = true;
                }
            }
        };
        let mut sim = Running::new(join2(
            serve_wide_pins(
                words,
                (arid, araddr, arlen, arvalid, rready),
                (arready_o, rid_o, rdata_o, rresp_o, rlast_o, rvalid_o),
                4,
            ),
            design,
        ));
        for c in 0..200u32 {
            now.set(c);
            sim.cycle();
        }
        let got = got.borrow();
        assert_eq!(got.len(), 2, "two beats");
        let asked = taken.get().expect("an address offered");
        assert_eq!(got[1].0, got[0].0 + 1, "one beat a step");
        assert!(
            got[0].0 - asked >= MODEL_READ_LATENCY,
            "the first beat at the model's latency: {got:?}, asked {asked}"
        );
        let low = [
            0x1000_0003_1000_0002_1000_0001_1000_0000u128,
            0x1000_0007_1000_0006_1000_0005_1000_0004,
        ];
        let high = [
            0x1000_000b_1000_000a_1000_0009_1000_0008u128,
            0x1000_000f_1000_000e_1000_000d_1000_000c,
        ];
        assert_eq!(got[0], (got[0].0, 7, low, false));
        assert_eq!(got[1], (got[1].0, 7, high, true));
    }
}
