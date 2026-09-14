// SPDX-License-Identifier: Apache-2.0
//! A FIFO with a channel at each end: the depth a channel's own
//! buffer of two does not have, for a unit whose consumer takes in
//! bursts or holds off.
use txhdl::comp::{mux, Clock, DefaultClock, Mem, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, Transaction, Value, U};
use txhdl::{lower, with, Trace};

/// A FIFO of `D` words of `T` between two channels, `D = 1 << AW`
/// stated, since a computed width in a type needs nightly Rust. A
/// word is taken from the input whenever there is room and the
/// oldest word is offered on the output whenever there is one. A
/// word taken lands in the memory at the end of its step and is
/// offered the step after, so the latency is one cycle; `full`
/// holds the input off, and the output's `ready` is the channel's.
/// Written in the lowered subset, so it is a netlist too.
#[derive(Trace, Default)]
pub struct Fifo<T: Transaction + Value, const AW: usize, const D: usize> {
    /// The words, `D` of them.
    pub mem: Mem<T, D>,
    /// Where the oldest word is, and where the next lands.
    pub head: Reg<U<AW>>,
    pub tail: Reg<U<AW>>,
    /// The pointers are equal when the FIFO is empty and when it is
    /// full; this says which.
    pub full: Reg<Bit>,
}

#[lower]
impl<T: Transaction + Value, const AW: usize, const D: usize> Unit
    for Fifo<T, AW, D>
{
    async fn run(&mut self, inp: Rx<T>, out: Tx<T>) {
        loop {
            DefaultClock::rising().await;
            let (head, tail) = (self.head.get(), self.tail.get());
            let full = self.full.get();
            let empty = (head == tail) & !full;
            // A word leaves when there is one and the output has room;
            // a word enters when one is offered and there is room.
            let pop = !empty & out.ready();
            let push = inp.peek().is_some() & !full;
            let word = inp.recv_if(!full).unwrap_or_default();
            // The next tail, as a wire, so its compare is `AW` wide.
            let tail1 = tail + 1;
            with!(self <= {
                push ? mem.at(tail): word,
                push ? tail: tail1,
                pop ? head: head + 1,
                full: mux(pop, Bit::Zero, full | (push & (tail1 == head))),
            });
            if pop.to_bool() {
                out.send(self.mem.read(head));
            }
        }
    }
}

/// The FIFO against a queue: every word sent comes out, in order, and
/// `full` is seen while the output is held.
#[cfg(test)]
mod tests {
    use super::Fifo;
    use std::collections::VecDeque;
    use txhdl::comp::{chan, DefaultClock, Running, Unit};
    use txhdl::types::U;

    #[test]
    fn fifo_keeps_every_word_in_order() {
        let (tx, a_rx) = chan::<U<8>, DefaultClock>();
        let (b_tx, rx) = chan::<U<8>, DefaultClock>();
        let mut fifo = Fifo::<U<8>, 2, 4>::default();
        let (full, head, tail) = (fifo.full, fifo.head, fifo.tail);
        let mut sim = Running::new(fifo.run(a_rx, b_tx));
        let mut sent: VecDeque<u8> = VecDeque::new();
        let mut next = 1u8;
        let mut x = 0x2545_F491u32;
        let mut saw_full = false;
        for t in 0..300 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            // Offers through the first two hundred cycles; takes after
            // a stall of forty, so the FIFO fills.
            let offer = t < 200 && (x & 3 != 0);
            let take = t >= 40 && (x & 4 != 0);
            if offer && tx.ready().to_bool() {
                tx.send(U::from(next));
                sent.push_back(next);
                next = next.wrapping_add(1);
            }
            if take {
                if let Some(v) = rx.recv() {
                    assert_eq!(v.raw() as u8, sent.pop_front().unwrap());
                }
            }
            sim.cycle();
            saw_full |= full.to_bool();
        }
        for _ in 0..12 {
            if let Some(v) = rx.recv() {
                assert_eq!(v.raw() as u8, sent.pop_front().unwrap());
            }
            sim.cycle();
        }
        assert!(saw_full, "the FIFO never filled");
        assert!(sent.is_empty(), "words left behind: {sent:?}");
        assert!(
            !full.to_bool() && head == tail.get(),
            "not empty at the end"
        );
    }
}
