// SPDX-License-Identifier: Apache-2.0
//! The stream ALU on its own: `StreamAlu` in `stream_alu.rs`, fed ten
//! requests from one source, its results taken out of order and each
//! checked. The unit is lowered, and the netlist is checked against
//! this run under both simulators.
use stream_alu::{expected, name, Req, Res, StreamAlu, ADD, AND, NOT, OR};
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, DefaultClock, Running, Unit};
use txhdl::types::U;

/// The requests of the run: an operation and its operands, tagged in
/// order. The adds carry across every nibble.
const REQUESTS: [(u8, u16, u16); 10] = [
    (ADD, 0x1234, 0x0FFF),
    (NOT, 0x00FF, 0),
    (AND, 0xF0F0, 0xFF00),
    (OR, 0x0F0F, 0x00F0),
    (ADD, 0xFFFF, 0x0001),
    (NOT, 0x1234, 0),
    (OR, 0x8000, 0x0001),
    (AND, 0xFFFF, 0x0F0F),
    (ADD, 0x0001, 0x0001),
    (NOT, 0xFFFF, 0),
];

fn main() {
    let (req_tx, req_rx) = chan::<Req, DefaultClock>();
    let (res_tx, res_rx) = chan::<Res, DefaultClock>();
    let mut alu = StreamAlu::default();
    let slots = alu.slots;
    let entered = [alu.not_v, alu.and1_v, alu.or1_v, alu.add1_v];
    let entered_tag = [alu.not_tag, alu.and1_tag, alu.or1_tag, alu.add1_tag];
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("inp", &req_rx);
        w.add("alu", &alu);
        w.add("out", &res_rx);
        w.start();
    }
    let mut sim = Running::new(alu.run(req_rx, res_tx));
    let mut next = 0;
    let mut results = 0;
    println!(" t sent     slots stage 1  result");
    for t in 0..24 {
        // The source sends the next request whenever the channel has
        // room; the sink takes every cycle but three.
        let sent = if next < REQUESTS.len() && req_tx.ready().to_bool() {
            let (op, a, b) = REQUESTS[next];
            req_tx.send(Req {
                tag: U::from(next),
                v0: U::from(op),
                v1: U::from(a),
                v2: U::from(b),
            });
            next += 1;
            format!("{:>2} {}", next - 1, name(op))
        } else {
            "-".to_string()
        };
        let taken = if (9..12).contains(&t) {
            None
        } else {
            res_rx.recv()
        };
        if let Some(r) = taken {
            let (op, a, b) = REQUESTS[r.tag.raw() as usize];
            assert_eq!(
                r.value.raw() as u16,
                expected(op, a, b),
                "tag {}",
                r.tag.raw()
            );
            results += 1;
        }
        sim.cycle();
        // What the first stage of each unit holds after the cycle: the
        // request taken in it, or the one held while the sink holds off.
        let stage1 = (0..4)
            .find(|&u| entered[u].get().to_bool())
            .map_or("-".to_string(), |u| {
                format!("{:>2} {}", entered_tag[u].get().raw(), name(u as u8))
            });
        println!(
            "{:2} {:8} {:04b} {:8}  {}",
            t,
            sent,
            slots.get().raw(),
            stage1,
            taken.map_or("-".to_string(), |r| format!(
                "tag {:>2} = {:#06x}",
                r.tag.raw(),
                r.value.raw()
            )),
        );
    }
    assert_eq!(results, REQUESTS.len(), "every request answered");
    stop();
    txhdl::netlist::write_vhdl_from_env(&StreamAlu::lowered("stream_alu"));
    print!("\n{}", StreamAlu::verilog("stream_alu"));
}
