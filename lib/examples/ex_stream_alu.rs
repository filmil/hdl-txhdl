// SPDX-License-Identifier: Apache-2.0
//! An ALU as a stream, pipelined, its results out of order. A request
//! is a tag, an operation and two operands on one channel; a result is
//! the tag and the value on another, in the order the units finish and
//! not the order the requests came. The four operations are four
//! units of four depths: `not` takes one cycle, `and` two, `or` three
//! and `add` four, a nibble per stage with the carry passed along. One
//! result can leave per cycle, so a request is taken only when the
//! cycle its unit would finish in is not already claimed: `slots`
//! holds a bit per cycle ahead and shifts down every cycle. The sink
//! holding off holds the whole pipeline. The unit is lowered, and
//! the netlist is checked against this run under both simulators.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, mux, Clock, DefaultClock, Reg, Running, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace, Transaction, Value};

// begin{unit}
/// The operations, numbered so that an operation's number is its
/// latency less one: the bit of `slots` it claims.
pub const NOT: u8 = 0;
pub const AND: u8 = 1;
pub const OR: u8 = 2;
pub const ADD: u8 = 3;

/// A request: which operation, on what, under which tag.
#[derive(Transaction, Value, Clone, Copy, Default, Debug)]
pub struct Req {
    pub tag: U<4>,
    pub op: U<2>,
    pub a: U<16>,
    pub b: U<16>,
}

/// A result: the tag of its request, and the value.
#[derive(Transaction, Value, Clone, Copy, Default, Debug)]
pub struct Res {
    pub tag: U<4>,
    pub value: U<16>,
}

/// The four units, stage by stage: a valid bit, the tag and the value
/// so far in every stage; the adder's stages also carry the operands
/// and the carry between nibbles.
#[derive(Trace, Default)]
pub struct StreamAlu {
    /// Bit `k` set: a result leaves `k + 1` cycles from now.
    pub slots: Reg<U<4>>,
    pub not_v: Reg<Bit>,
    pub not_tag: Reg<U<4>>,
    pub not_r: Reg<U<16>>,
    pub and1_v: Reg<Bit>,
    pub and1_tag: Reg<U<4>>,
    pub and1_r: Reg<U<16>>,
    pub and2_v: Reg<Bit>,
    pub and2_tag: Reg<U<4>>,
    pub and2_r: Reg<U<16>>,
    pub or1_v: Reg<Bit>,
    pub or1_tag: Reg<U<4>>,
    pub or1_r: Reg<U<16>>,
    pub or2_v: Reg<Bit>,
    pub or2_tag: Reg<U<4>>,
    pub or2_r: Reg<U<16>>,
    pub or3_v: Reg<Bit>,
    pub or3_tag: Reg<U<4>>,
    pub or3_r: Reg<U<16>>,
    pub add1_v: Reg<Bit>,
    pub add1_tag: Reg<U<4>>,
    pub add1_a: Reg<U<16>>,
    pub add1_b: Reg<U<16>>,
    pub add1_sum: Reg<U<16>>,
    pub add1_c: Reg<Bit>,
    pub add2_v: Reg<Bit>,
    pub add2_tag: Reg<U<4>>,
    pub add2_a: Reg<U<16>>,
    pub add2_b: Reg<U<16>>,
    pub add2_sum: Reg<U<16>>,
    pub add2_c: Reg<Bit>,
    pub add3_v: Reg<Bit>,
    pub add3_tag: Reg<U<4>>,
    pub add3_a: Reg<U<16>>,
    pub add3_b: Reg<U<16>>,
    pub add3_sum: Reg<U<16>>,
    pub add3_c: Reg<Bit>,
    pub add4_v: Reg<Bit>,
    pub add4_tag: Reg<U<4>>,
    pub add4_sum: Reg<U<16>>,
}

#[lower]
impl Unit for StreamAlu {
    async fn run(&mut self, inp: Rx<Req>, out: Tx<Res>) {
        loop {
            DefaultClock::rising().await;
            // What is offered, and the cycle its unit would finish in.
            let offered = inp.peek().is_some();
            let req = inp.head();
            let op = req.op;
            let tag = req.tag;
            let a = req.a;
            let b = req.b;
            let claimed = self.slots.get().bit(op.raw() as usize);
            // The sink's room moves every stage; without it all hold.
            let advance = out.ready();
            let take = offered & !claimed & advance;
            let _ = inp.recv_if(take);
            // The bit the request claims; it joins `slots` before the
            // shift, so next cycle it sits one nearer.
            let claim = U::<4>::from(1u8) << (op.raw() as usize);
            let take_not = take & (op == NOT);
            let take_and = take & (op == AND);
            let take_or = take & (op == OR);
            let take_add = take & (op == ADD);
            // The adder, a nibble per stage: the nibble's sum is five
            // bits, its top the carry into the next stage.
            let n0 =
                a.slice::<0, 4>().zext::<5>() + b.slice::<0, 4>().zext::<5>();
            let n1 = self.add1_a.get().slice::<4, 4>().zext::<5>()
                + self.add1_b.get().slice::<4, 4>().zext::<5>()
                + self.add1_c.get().zext::<5>();
            let n2 = self.add2_a.get().slice::<8, 4>().zext::<5>()
                + self.add2_b.get().slice::<8, 4>().zext::<5>()
                + self.add2_c.get().zext::<5>();
            let n3 = self.add3_a.get().slice::<12, 4>().zext::<5>()
                + self.add3_b.get().slice::<12, 4>().zext::<5>()
                + self.add3_c.get().zext::<5>();
            let sum1 = n0.slice::<0, 4>().zext::<16>();
            let sum2 = n1
                .slice::<0, 4>()
                .concat::<4, 8>(self.add1_sum.get().slice::<0, 4>())
                .zext::<16>();
            let sum3 = n2
                .slice::<0, 4>()
                .concat::<8, 12>(self.add2_sum.get().slice::<0, 8>())
                .zext::<16>();
            let sum4 = n3
                .slice::<0, 4>()
                .concat::<12, 16>(self.add3_sum.get().slice::<0, 12>());
            with!(self <= {
                advance ? {
                    slots: (self.slots.get()
                        | mux(take, claim, U::<4>::from(0u8)))
                        >> 1,
                    not_v: take_not,
                    not_tag: tag,
                    not_r: !a,
                    and1_v: take_and,
                    and1_tag: tag,
                    and1_r: a & b,
                    and2_v: self.and1_v,
                    and2_tag: self.and1_tag,
                    and2_r: self.and1_r,
                    or1_v: take_or,
                    or1_tag: tag,
                    or1_r: a | b,
                    or2_v: self.or1_v,
                    or2_tag: self.or1_tag,
                    or2_r: self.or1_r,
                    or3_v: self.or2_v,
                    or3_tag: self.or2_tag,
                    or3_r: self.or2_r,
                    add1_v: take_add,
                    add1_tag: tag,
                    add1_a: a,
                    add1_b: b,
                    add1_sum: sum1,
                    add1_c: n0.bit(4),
                    add2_v: self.add1_v,
                    add2_tag: self.add1_tag,
                    add2_a: self.add1_a,
                    add2_b: self.add1_b,
                    add2_sum: sum2,
                    add2_c: n1.bit(4),
                    add3_v: self.add2_v,
                    add3_tag: self.add2_tag,
                    add3_a: self.add2_a,
                    add3_b: self.add2_b,
                    add3_sum: sum3,
                    add3_c: n2.bit(4),
                    add4_v: self.add3_v,
                    add4_tag: self.add3_tag,
                    add4_sum: sum4,
                },
            });
            // At most one unit finishes per cycle, by `slots`; its
            // result leaves when the sink has room.
            let done = self.not_v | self.and2_v | self.or3_v | self.add4_v;
            let res_tag = mux(
                self.not_v,
                self.not_tag.get(),
                mux(
                    self.and2_v,
                    self.and2_tag.get(),
                    mux(self.or3_v, self.or3_tag.get(), self.add4_tag.get()),
                ),
            );
            let res_value = mux(
                self.not_v,
                self.not_r.get(),
                mux(
                    self.and2_v,
                    self.and2_r.get(),
                    mux(self.or3_v, self.or3_r.get(), self.add4_sum.get()),
                ),
            );
            if (done & advance).to_bool() {
                out.send(Res {
                    tag: res_tag,
                    value: res_value,
                });
            }
        }
    }
}
// end{unit}

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

fn name(op: u8) -> &'static str {
    ["not", "and", "or", "add"][op as usize]
}

/// What a request's result must be.
fn expected(op: u8, a: u16, b: u16) -> u16 {
    match op {
        NOT => !a,
        AND => a & b,
        OR => a | b,
        _ => a.wrapping_add(b),
    }
}

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
                op: U::from(op),
                a: U::from(a),
                b: U::from(b),
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
