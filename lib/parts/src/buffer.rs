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

#[derive(Trace, Default)]
pub struct Buffer<const W: usize> {
    pub head: Reg<U<W>>,
    pub head_full: Reg<Bit>,
    pub tail: Reg<U<W>>,
    pub tail_full: Reg<Bit>,
}

#[lower]
impl<const W: usize>
    Unit<(In<U<W>>, In<Bit>, In<Bit>), (Out<Bit>, Out<U<W>>, Out<Bit>)>
    for Buffer<W>
{
    async fn run(
        &mut self,
        (tx_data, tx_valid, rx_ready): (In<U<W>>, In<Bit>, In<Bit>),
        (tx_ready, rx_data, rx_valid): (Out<Bit>, Out<U<W>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let (tx_data, tx_valid) = (tx_data.get(), tx_valid.get());
            let rx_ready = rx_ready.get();
            let (head, head_full) = (self.head.get(), self.head_full.get());
            let (tail, tail_full) = (self.tail.get(), self.tail_full.get());
            // What the sides see this cycle, as the edge left it.
            tx_ready.set(tail_full.not());
            rx_data.set(head);
            rx_valid.set(head_full);
            // The take, then the push into whichever slot is free.
            let pop = rx_ready.and(head_full);
            let push = tx_valid.and(tail_full.not());
            let head1 = mux(pop, tail, head);
            let head_full1 = mux(pop, tail_full, head_full);
            let tail_full1 = mux(pop, Bit::Zero, tail_full);
            let to_head = push.and(head_full1.not());
            let to_tail = push.and(head_full1);
            self.head.set(mux(to_head, tx_data, head1));
            self.head_full.set(head_full1.or(to_head));
            self.tail.set(mux(to_tail, tx_data, tail));
            self.tail_full.set(tail_full1.or(to_tail));
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
    use txhdl::comp::{chan, signal, DefaultClock, Running, Unit};
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
        let (head, head_full, tail_full) = (
            buffer.head.clone(),
            buffer.head_full.clone(),
            buffer.tail_full.clone(),
        );
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
            tx_valid_o.set(Bit::from_bool(offer && room));
            rx_ready_o.set(Bit::from_bool(take));
            if offer && room {
                tx.send(word);
            }
            if take {
                let _ = rx.recv();
            }
            sim.cycle();
            assert_eq!(tail_full.get().not(), tx.ready(), "ready at {i}");
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
}
