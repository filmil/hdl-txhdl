// SPDX-License-Identifier: Apache-2.0
//! An ALU: the operators a datapath needs, each lowered. `op` selects
//! with `case!`, and the result is a register, so the lowering's
//! `case!` drives it and the trace checks it. The shifts take their
//! amount from the low bits of `b`, as RISC-V does; `slt` is the
//! signed compare, `sltu` the unsigned one.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{signal, Clock, DefaultClock, In, Out, Reg, Running, Unit};
use txhdl::funcs::{lt_signed, sra};
use txhdl::types::{Bit, U};
use txhdl::{case, lower, Trace, Value};

#[derive(Value, Clone, Copy, Default, PartialEq, Debug)]
pub enum Op {
    #[default]
    Add,
    Sub,
    And,
    Or,
    Xor,
    Sll,
    Srl,
    Sra,
    Slt,
    Sltu,
}

#[derive(Trace, Default)]
pub struct Alu {
    pub r: Reg<U<32>>,
}

#[lower]
impl Unit<(In<Op>, In<U<32>>, In<U<32>>), Out<U<32>>> for Alu {
    async fn run(
        &mut self,
        (op, a, b): (In<Op>, In<U<32>>, In<U<32>>),
        y: Out<U<32>>,
    ) {
        loop {
            DefaultClock::rising().await;
            let (op, a, b) = (op.get(), a.get(), b.get());
            case!(op => {
                Op::Add => { self.r <= a + b },
                Op::Sub => { self.r <= a - b },
                Op::And => { self.r <= a & b },
                Op::Or => { self.r <= a | b },
                Op::Xor => { self.r <= a ^ b },
                Op::Sll => { self.r <= a << (b.raw() as usize) },
                Op::Srl => { self.r <= a >> (b.raw() as usize) },
                Op::Sra => { self.r <= sra(a, b.raw() as usize) },
                Op::Slt => { self.r <= lt_signed(a, b).zext() },
                Op::Sltu => { self.r <= Bit::from(a < b).zext() },
            });
            y.set(self.r.get());
        }
    }
}

fn main() {
    let (op_out, op) = signal::<Op, DefaultClock>();
    let (a_out, a) = signal::<U<32>, DefaultClock>();
    let (b_out, b) = signal::<U<32>, DefaultClock>();
    let (y_out, y) = signal::<U<32>, DefaultClock>();
    let mut alu = Alu::default();
    let result = alu.r.clone();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("op", &op);
        w.add("a", &a);
        w.add("b", &b);
        w.add("alu", &alu);
        w.add("y", &y);
        w.start();
    }
    let mut sim = Running::new(alu.run((op, a, b), y_out));
    let table: [(Op, u32, u32); 10] = [
        (Op::Add, 7, 5),
        (Op::Sub, 7, 9),
        (Op::And, 0xF0F0, 0xFF00),
        (Op::Or, 0xF0F0, 0x0F0F),
        (Op::Xor, 0xFFFF, 0x0FF0),
        (Op::Sll, 1, 31),
        (Op::Srl, 0x8000_0000, 4),
        (Op::Sra, 0x8000_0000, 4),
        (Op::Slt, 0xFFFF_FFFF, 1),
        (Op::Sltu, 0xFFFF_FFFF, 1),
    ];
    for (o, x, z) in table {
        op_out.set(o);
        a_out.set(x);
        b_out.set(z);
        sim.cycle();
        println!("{:?} {:#x} {:#x} = {:#x}", o, x, z, result.get().raw());
    }
    sim.cycle();
    stop();
    txhdl::netlist::write_vhdl_from_env(&Alu::lowered("alu"));
    print!("\n{}", Alu::verilog("alu"));
}
