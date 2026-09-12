// SPDX-License-Identifier: Apache-2.0
//! The channel as hardware, checked against the channel as the runtime
//! has it. The testbench drives the buffer's three inputs each cycle
//! from a pattern of offers and takes, does the same offer and take on
//! a runtime channel, and asserts after every cycle that the buffer's
//! `ready`, `valid` and head are the channel's. So `Buffer` is the
//! runtime's elastic buffer of two, bit for bit, and its netlist is
//! checked against this trace under nvc and Verilator, which closes
//! the loop: a channel between two lowered units on a board behaves as
//! the channel between the two units in the run did.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, now, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::buffer::Buffer;

fn main() {
    let (tx_data_o, tx_data) = signal::<U<8>, DefaultClock>();
    let (tx_valid_o, tx_valid) = signal::<Bit, DefaultClock>();
    let (tx_ready_o, tx_ready) = signal::<Bit, DefaultClock>();
    let (rx_data_o, rx_data) = signal::<U<8>, DefaultClock>();
    let (rx_valid_o, rx_valid) = signal::<Bit, DefaultClock>();
    let (rx_ready_o, rx_ready) = signal::<Bit, DefaultClock>();
    let mut buffer = Buffer::<8>::default();
    // The buffer's state, as the edge leaves it, is what is compared:
    // its output wires are set in the step from the state before it,
    // and show the same a step later, which is the netlist check's
    // business.
    let (head, head_full, tail_full) = (
        buffer.head.clone(),
        buffer.head_full.clone(),
        buffer.tail_full.clone(),
    );
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("tx_data", &tx_data);
        w.add("tx_valid", &tx_valid);
        w.add("tx_ready", &tx_ready);
        w.add("rx_data", &rx_data);
        w.add("rx_valid", &rx_valid);
        w.add("rx_ready", &rx_ready);
        w.add("buffer", &buffer);
        w.start();
    }
    let mut sim = Running::new(buffer.run(
        (tx_data, tx_valid, rx_ready),
        (tx_ready_o, rx_data_o, rx_valid_o),
    ));
    // The same offers and takes on a runtime channel, the reference.
    let (tx, rx) = chan::<U<8>, DefaultClock>();
    // Offers on the cycles with a 1, takes on the cycles with a 2: the
    // buffer fills, holds the sender back, drains, and is offered and
    // taken in the same cycle both empty and full.
    let pattern: [u8; 14] = [1, 1, 1, 1, 3, 2, 2, 2, 3, 3, 1, 0, 2, 2];
    println!(" t offer take  ready valid head");
    for (i, &p) in pattern.iter().enumerate() {
        let (offer, take) = (p & 1 == 1, p & 2 == 2);
        // The inputs, set before the edge as a testbench sets them; an
        // offer only where there is room, which is the sender's rule.
        let room = tx.ready().to_bool();
        let word = U::<8>::from(10 + i as u8);
        tx_data_o.set(word);
        tx_valid_o.set(Bit::from_bool(offer && room));
        rx_ready_o.set(Bit::from_bool(take));
        if offer && tx.ready().to_bool() {
            tx.send(word);
        }
        if take {
            let _ = rx.recv();
        }
        sim.cycle();
        let (ready, valid) = (tail_full.get().not(), head_full.get());
        let head = head.get();
        println!(
            "{:>2} {:>5} {:>4}  {:>5} {:>5} {:>4}",
            now(),
            offer as u8,
            take as u8,
            ready.to_bool() as u8,
            valid.to_bool() as u8,
            head.raw()
        );
        assert_eq!(ready, tx.ready(), "ready at {}", now());
        assert_eq!(valid.to_bool(), rx.peek().is_some(), "valid at {}", now());
        if let Some(h) = rx.peek() {
            assert_eq!(head, h, "head at {}", now());
        }
    }
    stop();
    txhdl::netlist::write_vhdl_from_env(&Buffer::<8>::lowered("buffer8"));
    print!("\n{}", Buffer::<8>::verilog("buffer8"));
}
