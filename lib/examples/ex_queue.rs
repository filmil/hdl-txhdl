// SPDX-License-Identifier: Apache-2.0
//! A FIFO with a channel at each end: the `Fifo` part, four words
//! deep, between a source that offers a burst and a sink that holds
//! off until the FIFO is full and the source is held, then takes
//! every cycle. The table is the pointers and the full bit per cycle;
//! the netlist is checked against this run.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, now, DefaultClock, Running, Unit};
use txhdl::types::U;
use txhdl_parts::fifo::Fifo;

fn main() {
    let (tx, a_rx) = chan::<U<8>, DefaultClock>();
    let (b_tx, rx) = chan::<U<8>, DefaultClock>();
    let mut fifo = Fifo::<U<8>, 2, 4>::default();
    let (head, tail, full) = (fifo.head, fifo.tail, fifo.full);
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("inp", &a_rx);
        w.add("out", &rx);
        w.add("queue", &fifo);
        w.start();
    }
    let mut sim = Running::new(fifo.run(a_rx, b_tx));
    // Nine words offered back to back, a gap, three more; the sink
    // takes nothing for eight cycles, so the FIFO fills and holds the
    // source off, then takes every cycle.
    let offers = [1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0];
    let mut next = 1u8;
    println!(" t offer take  head tail full  out");
    for (t, &offer) in offers.iter().enumerate() {
        let offered = offer == 1 && tx.ready().to_bool();
        if offered {
            tx.send(U::from(next));
            next += 1;
        }
        let took = if t >= 8 { rx.recv() } else { None };
        sim.cycle();
        println!(
            "{:2} {:5} {:4}  {:4} {:4} {:4}  {}",
            now() / 2,
            offered as u8,
            took.is_some() as u8,
            head.get().raw(),
            tail.get().raw(),
            full.get().to_bool() as u8,
            took.map_or("-".to_string(), |v| v.raw().to_string())
        );
    }
    stop();
    txhdl::netlist::write_vhdl_from_env(&Fifo::<U<8>, 2, 4>::lowered("queue4"));
    print!("\n{}", Fifo::<U<8>, 2, 4>::verilog("queue4"));
}
