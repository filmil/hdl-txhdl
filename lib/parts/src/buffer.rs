// SPDX-License-Identifier: Apache-2.0
//! The channel as hardware: an elastic buffer of two, with the
//! runtime's rules exactly. A take moves the tail into the head; a
//! push goes into the head if it is empty after that, else into the
//! tail; `ready` on the sending side is the tail being empty as the
//! edge left it, `valid` on the receiving side the head being full.
//! Both are registers, so two units on a channel have no
//! combinational path between them, which is what the runtime
//! promises and what a board needs. One of these sits between two
//! lowered units wherever the run had a channel.
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

/// The language's channel as hardware: an elastic buffer of two, so
/// a sender and a receiver that both run every cycle pass one
/// transaction a cycle with `valid` and `ready` registered on both
/// sides and no combinational path between the two units.
///
/// `W` is the width of what it carries, in bits. A channel of a
/// struct is a channel of that struct's bits.
#[derive(Trace, Default)]
pub struct Buffer<const W: usize> {
    /// The older of the two entries: what the receiver is offered.
    pub head: Reg<U<W>>,
    /// Whether the head holds anything, which is the receiver's
    /// `valid`.
    pub head_full: Reg<Bit>,
    /// The younger entry, taken while the head waits to be read.
    pub tail: Reg<U<W>>,
    /// Whether the tail holds anything. Its absence is the sender's
    /// `ready`, so a full buffer is what stops a sender.
    pub tail_full: Reg<Bit>,
}

#[lower]
impl<const W: usize> Unit for Buffer<W> {
    async fn run(
        &mut self,
        (tx_data, tx_valid, rx_ready): (In<U<W>>, In<Bit>, In<Bit>),
        (tx_ready, rx_data, rx_valid): (Out<Bit>, Out<U<W>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let (tx_data, tx_valid) = (tx_data.get(), tx_valid.get());
            let rx_ready = rx_ready.get();
            // What the sides see this cycle, as the edge left it.
            tx_ready.set(!self.tail_full);
            rx_data.set(self.head);
            rx_valid.set(self.head_full);
            // The take, then the push into whichever slot is free.
            let pop = rx_ready & self.head_full;
            let push = tx_valid & !self.tail_full;
            let head1 = mux(pop, self.tail.get(), self.head.get());
            let head_full1 =
                mux(pop, self.tail_full.get(), self.head_full.get());
            let tail_full1 = mux(pop, Bit::Zero, self.tail_full.get());
            let to_head = push & !head_full1;
            let to_tail = push & head_full1;
            self.head.set(mux(to_head, tx_data, head1));
            self.head_full.set(head_full1 | to_head);
            self.tail.set(mux(to_tail, tx_data, self.tail.get()));
            self.tail_full.set(tail_full1 | to_tail);
        }
    }
}

/// A unit runs in a plain `#[test]` as it runs anywhere: the runtime's
/// clock and executor are per thread, so the tests of a crate run in
/// parallel as Rust runs them, each with a time of its own. This one
/// is the example's check in short: the buffer against a runtime
/// channel, on a pattern of offers and takes.
#[cfg(test)]
mod tests {
    use super::Buffer;
    use txhdl::comp::{chan, signal, Clock, DefaultClock, Running, Unit};
    use txhdl::types::{Bit, U};

    #[test]
    fn buffer_is_the_runtime_channel() {
        let (tx_data_o, tx_data) = signal::<U<8>, DefaultClock>();
        let (tx_valid_o, tx_valid) = signal::<Bit, DefaultClock>();
        let (tx_ready_o, _tx_ready) = signal::<Bit, DefaultClock>();
        let (rx_data_o, _rx_data) = signal::<U<8>, DefaultClock>();
        let (rx_valid_o, _rx_valid) = signal::<Bit, DefaultClock>();
        let (rx_ready_o, rx_ready) = signal::<Bit, DefaultClock>();
        let mut buffer = Buffer::<8>::default();
        let (head, head_full, tail_full) =
            (buffer.head, buffer.head_full, buffer.tail_full);
        let mut sim = Running::new(buffer.run(
            (tx_data, tx_valid, rx_ready),
            (tx_ready_o, rx_data_o, rx_valid_o),
        ));
        let (tx, rx) = chan::<U<8>, DefaultClock>();
        // A longer pattern than the example's, from a small generator.
        let mut r = 0x2545_f491_4f6c_dd1du64;
        for i in 0..200u32 {
            r ^= r << 13;
            r ^= r >> 7;
            r ^= r << 17;
            let (offer, take) = (r & 1 == 1, r & 2 == 2);
            let word = U::<8>::from(i as u8);
            let room = tx.ready().to_bool();
            tx_data_o.set(word);
            tx_valid_o.set(offer && room);
            rx_ready_o.set(take);
            if offer && room {
                tx.send(word);
            }
            if take {
                let _ = rx.recv();
            }
            sim.cycle();
            assert_eq!(!tail_full.get(), tx.ready(), "ready at {i}");
            assert_eq!(
                head_full.get().to_bool(),
                rx.peek().is_some(),
                "valid at {i}"
            );
            if let Some(h) = rx.peek() {
                assert_eq!(head.get(), h, "head at {i}");
            }
        }
    }

    /// One step is one offer. A channel has one `valid` and one
    /// `data`, so a second send in a step would replace the first and
    /// lose it; `ready` is the buffer as the edge left it and cannot
    /// say so. The runtime refuses it instead (issue 183).
    #[test]
    #[should_panic(expected = "two sends on one channel in one step")]
    fn a_second_send_in_one_step_is_refused() {
        let (tx, _rx) = chan::<U<8>, DefaultClock>();
        assert!(tx.ready().to_bool(), "the channel starts with room");
        tx.send(U::<8>::from(1u8));
        assert!(tx.ready().to_bool(), "ready still says the edge's room");
        tx.send(U::<8>::from(2u8));
    }

    /// A send in each of two steps is what the handshake is for, and
    /// the receiver sees both.
    #[test]
    fn a_send_in_each_step_is_taken() {
        let (tx, rx) = chan::<U<8>, DefaultClock>();
        let mut got = Vec::new();
        // A process that only waits, so the run has a step to take.
        let mut sim = Running::new(async {
            loop {
                DefaultClock::rising().await;
            }
        });
        for i in 1..=4u8 {
            if tx.ready().to_bool() {
                tx.send(U::<8>::from(i));
            }
            if let Some(v) = rx.recv() {
                got.push(v.raw() as u8);
            }
            sim.cycle();
        }
        assert_eq!(got, vec![1, 2, 3], "one a step, a step behind");
    }
}
