// SPDX-License-Identifier: Apache-2.0
//! A unit of units, lowered whole. A station of three inputs gathers
//! each request's operation and two operands, which arrive on three
//! channels in three orders, and the stream ALU behind it answers in
//! the order its units finish. `Top` holds the two as fields; its
//! `run` makes the channel between them and joins their `run`s, and
//! `#[lower]` reads that as one module with two instances, each the
//! child's own module. The netlist of the top is checked against
//! this run under both simulators, at its ports.
use stream_alu::{expected, name, Req, Res, StreamAlu, ADD, AND, NOT, OR};
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, join2, DefaultClock, Running, Rx, Tx, Unit};
use txhdl::types::U;
use txhdl::{lower, Trace};
use txhdl_parts::station::{Station3, Tagged};

// begin{unit}
/// The station's lines are the ALU's requests: four-bit tags, so
/// sixteen lines, the operation in the first cell and the operands
/// in the other two.
type Station = Station3<4, 16, U<2>, U<16>, U<16>>;

/// The two units and the channel between them.
#[derive(Trace, Default)]
pub struct Top {
    pub station: Station,
    pub alu: StreamAlu,
}

#[lower]
impl Unit for Top {
    async fn run(
        &mut self,
        (ops, xs, ys): (
            Rx<Tagged<4, U<2>>>,
            Rx<Tagged<4, U<16>>>,
            Rx<Tagged<4, U<16>>>,
        ),
        out: Tx<Res>,
    ) {
        let (req_tx, req_rx) = chan::<Req, DefaultClock>();
        join2(
            self.station.run((ops, xs, ys), req_tx),
            self.alu.run(req_rx, out),
        )
        .await;
    }
}
// end{unit}

/// The requests of the run, by tag: an operation and its operands.
const REQUESTS: [(u8, u16, u16); 8] = [
    (ADD, 0x1234, 0x0FFF),
    (NOT, 0x00FF, 0),
    (AND, 0xF0F0, 0xFF00),
    (OR, 0x0F0F, 0x00F0),
    (ADD, 0xFFFF, 0x0001),
    (NOT, 0x1234, 0),
    (OR, 0x8000, 0x0001),
    (AND, 0xFFFF, 0x0F0F),
];

fn main() {
    let (op_tx, op_rx) = chan::<Tagged<4, U<2>>, DefaultClock>();
    let (x_tx, x_rx) = chan::<Tagged<4, U<16>>, DefaultClock>();
    let (y_tx, y_rx) = chan::<Tagged<4, U<16>>, DefaultClock>();
    let (res_tx, res_rx) = chan::<Res, DefaultClock>();
    let mut top = Top::default();
    let (occ0, occ1, occ2) =
        (top.station.occ0, top.station.occ1, top.station.occ2);
    let slots = top.alu.slots;
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("ops", &op_rx);
        w.add("xs", &x_rx);
        w.add("ys", &y_rx);
        w.add("top", &top);
        w.add("out", &res_rx);
        w.start();
    }
    let mut sim = Running::new(top.run((op_rx, x_rx, y_rx), res_tx));
    // Each source offers the requests' pieces in an order of its own:
    // the operations in request order, the first operands from the
    // last request back, the second operands odd tags first; the
    // second source rests every third cycle. The sink takes every
    // cycle but three.
    let op_order = [0usize, 1, 2, 3, 4, 5, 6, 7];
    let x_order = [7usize, 6, 5, 4, 3, 2, 1, 0];
    let y_order = [1usize, 3, 5, 7, 0, 2, 4, 6];
    let (mut oi, mut xi, mut yi) = (0, 0, 0);
    let mut results = 0;
    println!(" t op     x    y    occ0 occ1 occ2 slots result");
    for t in 0..40 {
        let op = if oi < 8 && op_tx.ready().to_bool() {
            let tag = op_order[oi];
            let (op, _, _) = REQUESTS[tag];
            op_tx.send(Tagged {
                tag: U::from(tag),
                value: U::from(op),
            });
            oi += 1;
            format!("{tag} {}", name(op))
        } else {
            "-".to_string()
        };
        let x = if xi < 8 && t % 3 != 2 && x_tx.ready().to_bool() {
            let tag = x_order[xi];
            x_tx.send(Tagged {
                tag: U::from(tag),
                value: U::from(REQUESTS[tag].1),
            });
            xi += 1;
            format!("{tag}")
        } else {
            "-".to_string()
        };
        let y = if yi < 8 && y_tx.ready().to_bool() {
            let tag = y_order[yi];
            y_tx.send(Tagged {
                tag: U::from(tag),
                value: U::from(REQUESTS[tag].2),
            });
            yi += 1;
            format!("{tag}")
        } else {
            "-".to_string()
        };
        let taken = if (14..17).contains(&t) {
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
        println!(
            "{:2} {:6} {:4} {:4} {:04x} {:04x} {:04x} {:04b}  {}",
            t,
            op,
            x,
            y,
            occ0.get().raw(),
            occ1.get().raw(),
            occ2.get().raw(),
            slots.get().raw(),
            taken.map_or("-".to_string(), |r| format!(
                "tag {} = {:#06x}",
                r.tag.raw(),
                r.value.raw()
            )),
        );
    }
    assert_eq!(results, REQUESTS.len(), "every request answered");
    stop();
    txhdl::netlist::write_vhdl_from_env(&Top::lowered("top"));
    print!("\n{}", Top::verilog("top"));
}
