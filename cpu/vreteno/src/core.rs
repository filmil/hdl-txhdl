// SPDX-License-Identifier: Apache-2.0
//! Vreteno: a three-stage RV32IMC core. One process, and on every edge
//! three things at once: the fetch stage reads the instruction at the
//! program counter into the instruction register, sixteen bits or
//! thirty-two, a compressed one as the instruction it stands for, so
//! that nothing after the fetch knows there are two lengths; the execute
//! stage decodes the fields of the word in that register, executes,
//! stores, reads the data memory into a register at its edge, and, on
//! a taken branch or a jump, redirects the fetch and squashes the word
//! it fetched this cycle, which is the one-cycle penalty; and the
//! writeback stage extends a loaded word, writes the register file and
//! retires. An instruction in execute that reads what the one in
//! writeback has not yet written is given that value directly, the
//! forwarding path, unless that value is a load's, which lands too
//! late to forward: then the instruction waits one cycle, the one
//! stall there is. A trap or an `mret` is a redirect like a jump's.
//!
//! Written in the subset `#[lower]` reads: every value is a function
//! of the state and the inputs, `select!` chooses among values and
//! `with!` and `case!` among drives, and nothing branches. The pieces
//! that are functions of their operands alone, the immediates, the
//! ALU, the branch condition, the load's extension, the store's lanes,
//! the CSR access and the sequencer's result, are functions under
//! `#[lower]`, inlined where the step calls them. So the same file
//! simulates and lowers, and the netlist is simulated against the
//! trace the simulation wrote.
use crate::isa;
use txhdl::comp::{
    mux, Clock, DefaultClock, In, Mem, Out, Reg, Rx, Tx, Unit, Wire,
};
use txhdl::funcs::{lt_signed, sra};
use txhdl::types::{Bit, U};
use txhdl::{case, lower, select, when, with, Trace, Value};
use txhdl_parts::bus::axi::{BurstKind, Done, Grant, Issue, R, W};

/// Words of instruction memory. The data memory is a device on the
/// bus, `crate::dmem`, at `DATA_BASE` as the model has it.
pub const IMEM_WORDS: usize = 1024;
/// The boot memory's size in bytes, which is where it ends: a program
/// counter at or above this is fetched from the bus (issue 134).
pub const IMEM_BYTES: u32 = IMEM_WORDS as u32 * 4;
pub const DATA_BASE: u32 = crate::model::DATA_BASE;

/// What the core retired this cycle: `done` when an instruction
/// completed, and the register and the value it wrote, `rd` zero when
/// it wrote nothing. An output, so the waveform shows it, decoded
/// field by field, and the lockstep test steps the model on `done`.
#[derive(Value, Clone, Copy, Default, PartialEq, Debug)]
pub struct Writeback {
    pub done: Bit,
    pub rd: U<5>,
    pub val: U<32>,
}

// The pieces of the step that are functions of their operands alone,
// each under `#[lower]`, inlined by the lowering where the step calls
// it.

/// The I immediate: the top twelve bits, sign-extended.
#[lower]
fn imm_i(ir: U<32>) -> U<32> {
    ir.slice::<20, 12>().sext::<32>()
}

/// The S immediate, a store's offset, in two pieces.
#[lower]
fn imm_s(ir: U<32>) -> U<32> {
    ir.slice::<25, 7>()
        .concat::<_, 12>(ir.slice::<7, 5>())
        .sext::<32>()
}

/// The B immediate, a branch's offset, in four pieces and even.
#[lower]
fn imm_b(ir: U<32>) -> U<32> {
    ir.slice::<31, 1>()
        .concat::<_, 2>(ir.slice::<7, 1>())
        .concat::<_, 8>(ir.slice::<25, 6>())
        .concat::<_, 12>(ir.slice::<8, 4>())
        .concat::<_, 13>(U::<1>::from(0u8))
        .sext::<32>()
}

/// The U immediate: the top twenty bits, in place.
#[lower]
fn imm_u(ir: U<32>) -> U<32> {
    ir.slice::<12, 20>().concat::<_, 32>(U::<12>::from(0u8))
}

/// An I-type instruction from its fields.
#[lower]
fn enc_i(imm: U<12>, rs1: U<5>, f3: U<3>, rd: U<5>, op: U<7>) -> U<32> {
    imm.concat::<_, 17>(rs1)
        .concat::<_, 20>(f3)
        .concat::<_, 25>(rd)
        .concat::<_, 32>(op)
}

/// An S-type instruction, a store, from its fields.
#[lower]
fn enc_s(imm: U<12>, rs2: U<5>, rs1: U<5>, f3: U<3>) -> U<32> {
    let hi = imm.slice::<5, 7>();
    let lo = imm.slice::<0, 5>();
    hi.concat::<_, 12>(rs2)
        .concat::<_, 17>(rs1)
        .concat::<_, 20>(f3)
        .concat::<_, 25>(lo)
        .concat::<_, 32>(U::<7>::from(0x23u8))
}

/// An R-type instruction, or a shift by an immediate, from its fields.
#[lower]
fn enc_r(
    f7: U<7>,
    rs2: U<5>,
    rs1: U<5>,
    f3: U<3>,
    rd: U<5>,
    op: U<7>,
) -> U<32> {
    f7.concat::<_, 12>(rs2)
        .concat::<_, 17>(rs1)
        .concat::<_, 20>(f3)
        .concat::<_, 25>(rd)
        .concat::<_, 32>(op)
}

/// A jal from its offset, which is even, and its link register.
#[lower]
fn enc_j(off: U<21>, rd: U<5>) -> U<32> {
    let top = off.slice::<20, 1>();
    let low = off.slice::<1, 10>();
    let mid = off.slice::<11, 1>();
    let high = off.slice::<12, 8>();
    top.concat::<_, 11>(low)
        .concat::<_, 12>(mid)
        .concat::<_, 20>(high)
        .concat::<_, 25>(rd)
        .concat::<_, 32>(U::<7>::from(0x6fu8))
}

/// A beq or bne against x0, from its offset, which is even.
#[lower]
fn enc_b(off: U<13>, rs1: U<5>, f3: U<3>) -> U<32> {
    let top = off.slice::<12, 1>();
    let high = off.slice::<5, 6>();
    let low = off.slice::<1, 4>();
    let mid = off.slice::<11, 1>();
    top.concat::<_, 7>(high)
        .concat::<_, 12>(U::<5>::from(0u8))
        .concat::<_, 17>(rs1)
        .concat::<_, 20>(f3)
        .concat::<_, 24>(low)
        .concat::<_, 25>(mid)
        .concat::<_, 32>(U::<7>::from(0x63u8))
}

// begin{expand}
/// A compressed instruction as the thirty-two bit instruction it stands
/// for; a halfword that is none, as itself, which is no thirty-two bit
/// instruction either, since those have both low bits set, and so traps
/// as illegal with the halfword as its trap value. The key is the three
/// bits of the major opcode above the two that say compressed. A
/// register field of three bits names x8 to x15; x2 is the stack
/// pointer the short forms of the stack's loads and stores assume.
#[lower]
fn expand(h: U<16>) -> U<32> {
    let key = h.slice::<13, 3>().concat::<_, 5>(h.slice::<0, 2>());
    let rd = h.slice::<7, 5>();
    let rs2 = h.slice::<2, 5>();
    let rs1s = U::<2>::from(1u8).concat::<_, 5>(h.slice::<7, 3>());
    let rs2s = U::<2>::from(1u8).concat::<_, 5>(h.slice::<2, 3>());
    let x0 = U::<5>::from(0u8);
    let sp = U::<5>::from(2u8);
    // Bit 12, as a bit to concatenate and as a bit to test; a one-bit
    // value compared with a number is not VHDL that analyses, #160.
    let top = h.slice::<12, 1>();
    let top_set = h.bit(12);
    let op_imm = U::<7>::from(0x13u8);
    let op_op = U::<7>::from(0x33u8);
    // The immediates, each at the width its instruction takes.
    let imm6 = top.concat::<_, 6>(rs2).sext::<12>();
    let sh = rs2;
    let imm4spn = h
        .slice::<7, 4>()
        .concat::<_, 6>(h.slice::<11, 2>())
        .concat::<_, 7>(h.slice::<5, 1>())
        .concat::<_, 8>(h.slice::<6, 1>())
        .concat::<_, 10>(U::<2>::from(0u8))
        .zext::<12>();
    let imm16sp = top
        .concat::<_, 3>(h.slice::<3, 2>())
        .concat::<_, 4>(h.slice::<5, 1>())
        .concat::<_, 5>(h.slice::<2, 1>())
        .concat::<_, 6>(h.slice::<6, 1>())
        .concat::<_, 10>(U::<4>::from(0u8))
        .sext::<12>();
    let imm_lw = h
        .slice::<5, 1>()
        .concat::<_, 4>(h.slice::<10, 3>())
        .concat::<_, 5>(h.slice::<6, 1>())
        .concat::<_, 7>(U::<2>::from(0u8))
        .zext::<12>();
    let imm_lwsp = h
        .slice::<2, 2>()
        .concat::<_, 3>(top)
        .concat::<_, 6>(h.slice::<4, 3>())
        .concat::<_, 8>(U::<2>::from(0u8))
        .zext::<12>();
    let imm_swsp = h
        .slice::<7, 2>()
        .concat::<_, 6>(h.slice::<9, 4>())
        .concat::<_, 8>(U::<2>::from(0u8))
        .zext::<12>();
    let imm_lui = top.concat::<_, 6>(rs2).sext::<20>();
    let joff = top
        .concat::<_, 2>(h.slice::<8, 1>())
        .concat::<_, 4>(h.slice::<9, 2>())
        .concat::<_, 5>(h.slice::<6, 1>())
        .concat::<_, 6>(h.slice::<7, 1>())
        .concat::<_, 7>(h.slice::<2, 1>())
        .concat::<_, 8>(h.slice::<11, 1>())
        .concat::<_, 11>(h.slice::<3, 3>())
        .concat::<_, 12>(U::<1>::from(0u8))
        .sext::<21>();
    let boff = top
        .concat::<_, 3>(h.slice::<5, 2>())
        .concat::<_, 4>(h.slice::<2, 1>())
        .concat::<_, 6>(h.slice::<10, 2>())
        .concat::<_, 8>(h.slice::<3, 2>())
        .concat::<_, 9>(U::<1>::from(0u8))
        .sext::<13>();
    // The instructions, one per group of the major opcode.
    let lui_rd = imm_lui
        .concat::<_, 25>(rd)
        .concat::<_, 32>(U::<7>::from(0x37u8));
    let addi16sp = enc_i(imm16sp, sp, U::<3>::from(0u8), sp, op_imm);
    let arith_f3 = select!(h.slice::<5, 2>().raw() => {
        0 => U::<3>::from(0u8),
        1 => U::<3>::from(4u8),
        2 => U::<3>::from(6u8),
        _ => U::<3>::from(7u8),
    });
    let arith_f7 = mux(
        h.slice::<5, 2>() == 0,
        U::<7>::from(0x20u8),
        U::<7>::from(0u8),
    );
    // The function codes the instructions below take. One name to a
    // let: a tuple bound in a lowered function is not declared in its
    // netlist, #159.
    let f0 = U::<3>::from(0u8);
    let f1 = U::<3>::from(1u8);
    let f2 = U::<3>::from(2u8);
    let f5 = U::<3>::from(5u8);
    let f7 = U::<3>::from(7u8);
    let plain = U::<7>::from(0u8);
    let alt = U::<7>::from(0x20u8);
    let op_load = U::<7>::from(0x03u8);
    let srli = enc_r(plain, sh, rs1s, f5, rs1s, op_imm);
    let srai = enc_r(alt, sh, rs1s, f5, rs1s, op_imm);
    let andi = enc_i(imm6, rs1s, f7, rs1s, op_imm);
    let arith = enc_r(arith_f7, rs2s, rs1s, arith_f3, rs1s, op_op);
    let misc = select!(h.slice::<10, 2>().raw() => {
        0 => srli,
        1 => srai,
        2 => andi,
        _ => arith,
    });
    let no_rs2 = rs2 == 0;
    let jalr_rd = mux(top_set, U::<5>::from(1u8), x0);
    let jumps = mux(
        no_rs2,
        mux(
            top_set & (rd == 0),
            U::<32>::from(0x0010_0073u32),
            enc_i(
                U::<12>::from(0u8),
                rd,
                U::<3>::from(0u8),
                jalr_rd,
                U::<7>::from(0x67u8),
            ),
        ),
        enc_r(
            U::<7>::from(0u8),
            rs2,
            mux(top_set, rd, x0),
            U::<3>::from(0u8),
            rd,
            op_op,
        ),
    );
    let addi4spn = enc_i(imm4spn, sp, f0, rs2s, op_imm);
    let lw = enc_i(imm_lw, rs1s, f2, rs2s, op_load);
    let sw = enc_s(imm_lw, rs2s, rs1s, f2);
    let addi = enc_i(imm6, rd, f0, rd, op_imm);
    let jal = enc_j(joff, U::<5>::from(1u8));
    let li = enc_i(imm6, x0, f0, rd, op_imm);
    let upper = mux(rd == 2, addi16sp, lui_rd);
    let j = enc_j(joff, x0);
    let beqz = enc_b(boff, rs1s, f0);
    let bnez = enc_b(boff, rs1s, f1);
    let slli = enc_r(plain, sh, rd, f1, rd, op_imm);
    let lwsp = enc_i(imm_lwsp, sp, f2, rd, op_load);
    let swsp = enc_s(imm_swsp, rs2, sp, f2);
    let word = select!(key.raw() => {
        0 => addi4spn,
        8 => lw,
        24 => sw,
        1 => addi,
        5 => jal,
        9 => li,
        13 => upper,
        17 => misc,
        21 => j,
        25 => beqz,
        29 => bnez,
        2 => slli,
        10 => lwsp,
        18 => jumps,
        _ => swsp,
    });
    // Whether the halfword is an instruction: what the specification
    // reserves, and what RV32C does not have, is not.
    let ok = select!(key.raw() => {
        0 => (h.slice::<5, 8>() != 0).into(),
        8 | 24 | 1 | 5 | 9 | 21 | 25 | 29 | 26 => Bit::One,
        13 => top_set | (rs2 != 0),
        17 => !top_set | (h.slice::<10, 2>() == 2),
        2 => !top_set,
        10 => (rd != 0).into(),
        18 => top_set | (rd != 0) | (rs2 != 0),
        _ => Bit::Zero,
    });
    mux(ok, word, h.zext::<32>())
}
// end{expand}

/// The J immediate, a jump's offset, in four pieces and even.
#[lower]
fn imm_j(ir: U<32>) -> U<32> {
    ir.slice::<31, 1>()
        .concat::<_, 9>(ir.slice::<12, 8>())
        .concat::<_, 10>(ir.slice::<20, 1>())
        .concat::<_, 20>(ir.slice::<21, 10>())
        .concat::<_, 21>(U::<1>::from(0u8))
        .sext::<32>()
}

// begin{alu}
/// The ALU: ten operations by `f3`, with `sub` telling subtract from
/// add and the arithmetic shift from the logical.
#[lower]
fn alu(f3: U<3>, sub: Bit, a: U<32>, b: U<32>) -> U<32> {
    let sh = b.slice::<0, 5>();
    select!(f3.raw() => {
        0 => mux(sub, a - b, a + b),
        1 => a << (sh.raw() as usize),
        2 => lt_signed(a, b).zext(),
        3 => Bit::from(a < b).zext(),
        4 => a ^ b,
        5 => mux(sub, sra(a, sh.raw() as usize), a >> (sh.raw() as usize)),
        6 => a | b,
        _ => a & b,
    })
}
// end{alu}

/// Whether a branch is taken, by `f3`.
#[lower]
fn branch(f3: U<3>, a: U<32>, b: U<32>) -> Bit {
    select!(f3.raw() => {
        0 => (a == b).into(),
        1 => (a != b).into(),
        4 => lt_signed(a, b),
        5 => !lt_signed(a, b),
        6 => (a < b).into(),
        _ => (a >= b).into(),
    })
}

/// A loaded word's byte or half, by the lane and the width the load
/// read with, extended; or the word itself.
#[lower]
fn extended(f3: U<3>, lane: U<2>, word: U<32>) -> U<32> {
    let bsh = lane.concat::<_, 5>(U::<3>::from(0u8));
    let hsh = lane.slice::<1, 1>().concat::<_, 5>(U::<4>::from(0u8));
    let octet = (word >> (bsh.raw() as usize)).slice::<0, 8>();
    let half = (word >> (hsh.raw() as usize)).slice::<0, 16>();
    select!(f3.raw() => {
        0 => octet.sext::<32>(),
        1 => half.sext::<32>(),
        2 => word,
        4 => octet.zext::<32>(),
        _ => half.zext::<32>(),
    })
}

/// What each lane takes on a store, the highest lane first: its byte
/// of the word for a word, the half's byte for a half, the byte for a
/// byte.
#[lower]
fn store_data(f3: U<3>, b: U<32>) -> U<32> {
    let b0 = b.slice::<0, 8>();
    let b1 = b.slice::<8, 8>();
    let b2 = b.slice::<16, 8>();
    let b3 = b.slice::<24, 8>();
    let d1 = select!(f3.raw() => { 0 => b0, _ => b1 });
    let d2 = select!(f3.raw() => { 2 => b2, _ => b0 });
    let d3 = select!(f3.raw() => { 0 => b0, 2 => b3, _ => b1 });
    d3.concat::<_, 16>(d2)
        .concat::<_, 24>(d1)
        .concat::<_, 32>(b0)
}

/// Which lanes a store writes, the highest lane first: one for a
/// byte, two for a half, all four for a word.
#[lower]
fn store_lanes(f3: U<3>, lane: U<2>) -> U<4> {
    let upper = lane.bit(1);
    let en0 = select!(f3.raw() => {
        0 => (lane == 0).into(),
        1 => !upper,
        _ => Bit::One,
    });
    let en1 = select!(f3.raw() => {
        0 => (lane == 1).into(),
        1 => !upper,
        _ => Bit::One,
    });
    let en2 = select!(f3.raw() => {
        0 => (lane == 2).into(),
        1 => upper,
        _ => Bit::One,
    });
    let en3 = select!(f3.raw() => {
        0 => (lane == 3).into(),
        1 => upper,
        _ => Bit::One,
    });
    en3.zext::<1>()
        .concat::<_, 2>(en2.zext::<1>())
        .concat::<_, 3>(en1.zext::<1>())
        .concat::<_, 4>(en0.zext::<1>())
}

/// A CSR read, by its number; `mip` is the pending register as the
/// core shows it, with the timer's line in it.
///
/// Each parameter is a wire of the unit, so there are as many of them
/// as there are registers to choose between; a struct of them is not
/// something the lowering reads.
#[allow(clippy::too_many_arguments)]
#[lower]
fn csr_read(
    f12: U<12>,
    mstatus: U<32>,
    mtvec: U<32>,
    mscratch: U<32>,
    mepc: U<32>,
    mcause: U<32>,
    mie: U<32>,
    mip: U<32>,
    mtval: U<32>,
) -> U<32> {
    select!(f12.raw() => {
        0x300 => mstatus,
        0x305 => mtvec,
        0x340 => mscratch,
        0x341 => mepc,
        0x342 => mcause,
        0x304 => mie,
        0x344 => mip,
        0x343 => mtval,
        0x301 => U::<32>::from(isa::MISA),
        // `mvendorid`, `marchid`, `mimpid` and `mhartid` all read as
        // zero, which the default below gives them, and so does the
        // halt. What makes them legal rather than illegal is
        // `csr_known`, not an arm here.
        _ => U::<32>::from(0u32),
    })
}

/// Whether a CSR number is one the core has. Eight hold a trap and
/// return from it; `mhalt` at `0x7c0` is this core's own, a write of
/// an odd value to which stops the machine, and it reads as zero; and
/// six say what the machine is, `misa` and the four machine
/// information registers, which is what a stock kernel reads before it
/// does anything else (issue 274).
#[lower]
fn csr_known(f12: U<12>) -> Bit {
    select!(f12.raw() => {
        0x300 | 0x305 | 0x340 | 0x341 | 0x342 | 0x304 | 0x344
        | 0x343 | 0x7c0 | 0x301 | 0xf11 | 0xf12 | 0xf13
        | 0xf14 => Bit::One,
        _ => Bit::Zero,
    })
}

/// Whether a CSR number is one of the read-only ones, whose address
/// begins with two set bits. A write to one is an illegal
/// instruction; a read is not.
// The four addresses are consecutive, so Clippy asks for a range. The
// lowering reads a `select!` arm's alternatives one by one and not a
// range, which is the rule `AGENTS.md` states, so they are written out.
#[allow(clippy::manual_range_patterns)]
#[lower]
fn csr_ro(f12: U<12>) -> Bit {
    select!(f12.raw() => {
        0xf11 | 0xf12 | 0xf13 | 0xf14 => Bit::One,
        _ => Bit::Zero,
    })
}

/// A CSR's new value: replaced, set or cleared by the source, which
/// is a register or a five-bit immediate.
#[lower]
fn csr_value(f3: U<3>, old: U<32>, src: U<32>) -> U<32> {
    select!(f3.raw() => {
        1 | 5 => src,
        2 | 6 => old | src,
        _ => old & !src,
    })
}

/// Whether an M operation takes its first operand as signed: every
/// one but the unsigned multiply-highs, divide and remainder.
#[lower]
fn m_signed_a(f3: U<3>) -> bool {
    (f3 == 0) | (f3 == 1) | (f3 == 2) | (f3 == 4) | (f3 == 6)
}

/// Whether an M operation takes its second operand as signed.
#[lower]
fn m_signed_b(f3: U<3>) -> bool {
    (f3 == 0) | (f3 == 1) | (f3 == 4) | (f3 == 6)
}

/// An M result from the sequencer's registers, with the signs put
/// back: a product or a quotient is negated when the signs differed,
/// a remainder takes the dividend's sign, and a quotient by zero is
/// all ones.
// A `select!` arm's alternatives are compared one by one when the
// function is lowered, so they are written out rather than as a range.
#[lower]
#[allow(clippy::manual_range_patterns)]
fn m_result(f3: U<3>, hi: U<33>, lo: U<32>, neg_q: Bit, neg_r: Bit) -> U<32> {
    let mag = hi.slice::<0, 32>().concat::<_, 64>(lo);
    let p = mux(neg_q, U::<64>::from(0u32) - mag, mag);
    let q = mux(neg_q, U::<32>::from(0u32) - lo, lo);
    let rem = hi.slice::<0, 32>();
    let r = mux(neg_r, U::<32>::from(0u32) - rem, rem);
    select!(f3.raw() => {
        0 => p.slice::<0, 32>(),
        1 | 2 | 3 => p.slice::<32, 32>(),
        4 | 5 => q,
        _ => r,
    })
}

/// The data memory is four memories of a byte, one per lane of the
/// word, each with a write port of its own, so a store of a byte or a
/// half writes its lanes and reads nothing, and the one read of the
/// four lanes feeds a register: what a block RAM is.
///
/// The state. The fetch stage's program counter. The instruction
/// register, its program counter and whether it holds an instruction,
/// the boundary between fetch and execute. The writeback registers,
/// the boundary between execute and writeback: whether one is there,
/// its destination, its value if not a load, the raw word read for a
/// load with the lane and the width to take from it, and whether it
/// halts. The halt itself, which the writeback stage sets. And the
/// three memories.
#[derive(Trace, Default)]
pub struct Vreteno<const IW: usize> {
    pub pc: Reg<U<32>>,
    pub ir: Reg<U<32>>,
    pub ir_c: Reg<Bit>,
    pub ir_pc: Reg<U<32>>,
    pub valid: Reg<Bit>,
    pub stopped: Reg<Bit>,
    pub wb_valid: Reg<Bit>,
    pub wb_pc: Reg<U<32>>,
    pub wb_ir: Reg<U<32>>,
    pub wb_rd: Reg<U<5>>,
    pub wb_alu: Reg<U<32>>,
    pub wb_f3: Reg<U<3>>,
    pub wb_lane: Reg<U<2>>,
    pub wb_load: Reg<Bit>,
    pub wb_stop: Reg<Bit>,
    pub halted: Reg<Bit>,
    pub mstatus: Reg<U<32>>,
    pub mtvec: Reg<U<32>>,
    pub mscratch: Reg<U<32>>,
    pub mepc: Reg<U<32>>,
    pub mcause: Reg<U<32>>,
    pub mie: Reg<U<32>>,
    pub mip: Reg<U<32>>,
    pub mtval: Reg<U<32>>,
    pub wb_dev: Reg<U<32>>,
    pub dev_wait: Reg<Bit>,
    /// Three of the cycle's decisions, kept as wires so that a trace
    /// shows them: whether the instruction in execute is stalled, whether
    /// an interrupt takes its place, and whether the fetch is redirected.
    pub stall: Wire<Bit>,
    pub int_take: Wire<Bit>,
    pub redirect: Wire<Bit>,
    pub m_busy: Reg<Bit>,
    pub m_count: Reg<U<6>>,
    pub m_hi: Reg<U<33>>,
    pub m_lo: Reg<U<32>>,
    pub m_d: Reg<U<32>>,
    pub m_neg_q: Reg<Bit>,
    pub m_neg_r: Reg<Bit>,
    pub regs: Mem<U<32>, 32>,
    pub imem: Mem<U<32>, IMEM_WORDS>,
    /// A fetch that is out on the bus, for a program above the boot
    /// memory: whether one is out, the word it asked for, the two
    /// words it has brought back, how many of them, and the address
    /// they start at.
    pub f_wait: Reg<Bit>,
    pub f_asked: Reg<U<32>>,
    pub f_w0: Reg<U<32>>,
    pub f_w1: Reg<U<32>>,
    pub f_have: Reg<U<2>>,
    pub f_at: Reg<U<32>>,
    /// Whether the fetch that is out is for the second word of a
    /// thirty-two bit instruction that straddles two words.
    pub f_second: Reg<Bit>,
}

impl<const IW: usize> Vreteno<IW> {
    /// A core with its program loaded.
    pub fn with(program: &[u32]) -> Self {
        let words: Vec<U<32>> = program.iter().map(|&w| U::from(w)).collect();
        Vreteno {
            imem: Mem::with(&words),
            ..Default::default()
        }
    }

    /// The architectural program counter: that of the oldest
    /// instruction not yet retired, in writeback, else in execute,
    /// else the fetch's. What the model's program counter is compared
    /// against.
    pub fn arch_pc(&self) -> U<32> {
        if self.wb_valid.get().to_bool() {
            self.wb_pc.get()
        } else if self.valid.get().to_bool() {
            self.ir_pc.get()
        } else {
            self.pc.get()
        }
    }
}

#[lower]
impl<const IW: usize> Unit for Vreteno<IW> {
    async fn run(
        &mut self,
        (rst, irq, tirq, sirq, rdata, done, grant): (
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
            Rx<R<32, IW>>,
            Rx<Done<IW>>,
            Rx<Grant<IW>>,
        ),
        (halt, instr, wb, issue, wbeat, release): (
            Out<Bit>,
            Out<U<32>>,
            Out<Writeback>,
            Tx<Issue<32>>,
            Tx<W<32, 4>>,
            Tx<Grant<IW>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let (rst, irq, tirq) = (rst.get(), irq.get(), tirq.get());
            let sirq = sirq.get();
            // The registers a step takes apart or hands on as values;
            // the rest are read where they are used.
            let fetch_pc = self.pc.get();
            let (ir, pc) = (self.ir.get(), self.ir_pc.get());
            let (wb_rd, wb_alu) = (self.wb_rd.get(), self.wb_alu.get());
            let (mstatus, mtvec, mepc) =
                (self.mstatus.get(), self.mtvec.get(), self.mepc.get());
            let (mie_r, mip) = (self.mie.get(), self.mip.get());
            // The bus's answers. A write's response and a load's beat
            // come back on channels of their own and one identifier
            // goes back a cycle, so a write's is taken first and a
            // load's waits. That cannot starve the load: a load holds
            // the core in `dev_wait`, so no further store is issued,
            // and the stores already out are as many as there are
            // identifiers.
            let rel_room = release.ready();
            let dh = done.head();
            let dv = done.peek().is_some();
            let rh = rdata.head();
            let take_done = dv & rel_room;
            let take_r = rdata.peek().is_some() & rel_room & !dv;
            let _ = done.recv_if(rel_room);
            let _ = rdata.recv_if(rel_room & !dv);
            let resp_valid = take_r;
            let resp_data = rh.data;
            let (m_hi, m_lo, m_d) =
                (self.m_hi.get(), self.m_lo.get(), self.m_d.get());
            // The writeback stage: a loaded word's byte or half, by the
            // lane and the width the execute stage read with it,
            // extended; else the value execute computed. Written to the
            // register file, and forwarded to execute below.
            // A load's word: what the bus answered, in its register.
            let loaded = extended(
                self.wb_f3.get(),
                self.wb_lane.get(),
                self.wb_dev.get(),
            );
            let wb_val = mux(self.wb_load, loaded, wb_alu);
            // A device load sits in writeback while its wait is on, and
            // retires the cycle after its answer has landed.
            let wb_here = self.wb_valid & !self.dev_wait;
            let wb_write = wb_here & (wb_rd != 0);
            // The fetch stage: the instruction at the program counter,
            // into the instruction register unless the execute stage
            // redirects below. The memory holds little-endian words, so
            // the halfword at an address with bit 1 set is the upper
            // half of its word; a compressed instruction is that
            // halfword alone, and a thirty-two bit one takes its upper
            // half from the halfword after, which may be the next word.
            let widx = fetch_pc.slice::<2, 10>();
            // The program counter is above the boot memory: the words
            // come from the bus instead, into a buffer of two, since a
            // thirty-two bit instruction at an odd halfword takes its
            // upper half from the word after.
            let far = fetch_pc >= U::<32>::from(IMEM_BYTES);
            let want = fetch_pc & U::<32>::from(0xffff_fffcu32);
            let f_at = self.f_at.get();
            let f_have = self.f_have.get();
            let hit0 = (f_have != 0) & (f_at == want);
            let hit1 = (f_have == 2) & (f_at == want);
            let w0 = mux(far, self.f_w0.get(), self.imem.read(widx));
            let w1 = mux(far, self.f_w1.get(), self.imem.read(widx + 1));
            let odd = fetch_pc.bit(1);
            let lo = mux(odd, w0.slice::<16, 16>(), w0.slice::<0, 16>());
            let hi = mux(odd, w1.slice::<0, 16>(), w0.slice::<16, 16>());
            let short = lo.slice::<0, 2>() != 3;
            let expanded = expand(lo);
            let fetched = mux(short, expanded, hi.concat::<_, 32>(lo));
            // The execute stage: the fields of the word.
            let opcode = ir.slice::<0, 7>();
            let rd = ir.slice::<7, 5>();
            let f3 = ir.slice::<12, 3>();
            let rs1 = ir.slice::<15, 5>();
            let rs2 = ir.slice::<20, 5>();
            let alt = ir.bit(30);
            // The five immediates, each a sign-extended word.
            let imm_i = imm_i(ir);
            let imm_s = imm_s(ir);
            let imm_b = imm_b(ir);
            let imm_u = imm_u(ir);
            let imm_j = imm_j(ir);
            // The operands: register zero reads as zero, and a register
            // the writeback stage is about to write reads as the value it
            // will write, which is the forwarding path. A load is the
            // exception: its word lands at the edge and is extended in
            // writeback, too long a path to forward through the ALU into
            // the fetch, so an instruction that reads what the load in
            // writeback will write waits one cycle and reads the register
            // file, which has it by then. The stall is the one cycle the
            // core ever waits.
            let fwd_a = wb_write & (wb_rd == rs1);
            let fwd_b = wb_write & (wb_rd == rs2);
            let stall_ld = self.valid & self.wb_load & (fwd_a | fwd_b);
            // The M extension is a sequencer: a multiply is one step in
            // the part's multipliers, a division thirty-two restoring
            // steps, over the magnitudes, and the instruction stalls in
            // execute until the count is up. One set of registers serves
            // both.
            let f7 = ir.slice::<25, 7>();
            let is_m = (opcode == 0x33) & (f7 == 1);
            let m_here = self.valid & !rst & !self.stopped & is_m;
            let m_done = self.m_busy & (self.m_count == 32);
            // An interrupt is taken instead of the instruction in execute
            // when it is pending, enabled, and interrupts are enabled; an
            // M instruction under one does not start, so it cannot hold
            // the stall the interrupt waits for.
            // The timer's line is a register's output on the timer's
            // side, so it is read as state is.
            let mtip = tirq;
            let ext_ok = mie_r.bit(11) & mip.bit(11);
            let soft_ok = mie_r.bit(3) & sirq;
            let tim_ok = mie_r.bit(7) & mtip;
            let int_ok = mstatus.bit(3) & (ext_ok | soft_ok | tim_ok);
            let stall_m = m_here & !int_ok & !m_done;
            // A load or store to the bus, which is everything above the
            // data memory. A store goes out when the bus has room; a
            // load goes out and moves on to writeback, which holds it
            // until the answer has landed in its register there.
            let a = mux(
                rs1 == 0,
                U::<32>::from(0u32),
                mux(fwd_a, wb_alu, self.regs.read(rs1)),
            );
            let b = mux(
                rs2 == 0,
                U::<32>::from(0u32),
                mux(fwd_b, wb_alu, self.regs.read(rs2)),
            );
            let addr = a + select!(opcode.raw() => {
                0x23 => imm_s,
                _ => imm_i,
            });
            let is_load = opcode == 0x03;
            let is_store = opcode == 0x23;
            // A half wants an even address and a word one that is a
            // multiple of four; a byte is never misaligned. The
            // specification lets a core either support an unaligned
            // access or raise the exception. This one raises it: it
            // used to take the aligned word and say nothing, which was
            // issue 138, and a wrong answer nothing reports is worse
            // than a trap a handler can emulate.
            let bad_half = ((f3 == 1) | (f3 == 5)) & addr.bit(0);
            let bad_word = (f3 == 2) & (addr.slice::<0, 2>() != 0);
            let unaligned = (is_load | is_store) & (bad_half | bad_word);
            // Everything from the data memory up is on the bus.
            let is_dev = addr.slice::<12, 20>() != 0;
            let here = self.valid & !rst & !self.stopped;
            // A load or a store waits for room on the bus whatever its
            // address, so that the fetch's hold does not hang on the
            // address's decode, which was the path that limited the
            // clock; there is room whenever the devices keep up, and
            // they do. A device load's wait for its answer is a
            // register, so the hold for it is state too.
            let stall_bus = here
                & (is_load | is_store)
                & !unaligned
                & !(issue.ready() & wbeat.ready());
            // A word of the instruction is still on the bus: the core
            // waits for it, which is what makes a program above the
            // boot memory slow and correct.
            let need1 = far & odd & !short;
            let f_ready = !far | (hit0 & (!need1 | hit1));
            let stall_fetch = !f_ready;
            self.stall.set(
                stall_ld | stall_m | stall_bus | stall_fetch | self.dev_wait,
            );
            let stall = self.stall.get();
            // The address after the instruction, which a jump links:
            // two bytes on for a compressed one, four for the rest.
            let link =
                pc + mux(self.ir_c, U::<32>::from(2u32), U::<32>::from(4u32));
            // Live: an instruction in execute that is not stalled. It
            // runs unless the interrupt takes its place.
            let live = !rst & !self.stopped & self.valid & !stall;
            self.int_take.set(live & int_ok);
            let int_take = self.int_take.get();
            let run = live & !int_take;
            let send_load = run & is_load & is_dev & !unaligned;

            // The ALU, shared by the register and immediate forms; bit
            // 30 means subtract or arithmetic shift, except that an
            // immediate may have it set and mean nothing by it.
            let alu_b = mux(opcode == 0x13, imm_i, b);
            let sub = alt & ((opcode == 0x33) | (f3 == 5));
            // The multiply and divide, from the sequencer's registers.
            let m_res = m_result(
                f3,
                m_hi,
                m_lo,
                self.m_neg_q.get(),
                self.m_neg_r.get(),
            );
            let alu = alu(f3, sub, a, alu_b);
            // The branch condition.
            let taken = branch(f3, a, b);
            // Loads and stores go out on the bus: the lane within the
            // word, what each lane takes on a store, and which lanes.
            let lane = addr.slice::<0, 2>();
            // What each lane takes on a store, and which lanes take it.
            let sdata = store_data(f3, b);
            let en = store_lanes(f3, lane);

            // What the instruction does: the value it writes back, if
            // any, where it goes next, and whether the core knows it.
            let writes = select!(opcode.raw() => {
                0x37 | 0x17 | 0x6f | 0x67 | 0x03 | 0x13 | 0x33 => Bit::One,
                0x73 => (f3 != 0).into(),
                _ => Bit::Zero,
            });
            // The system instructions. A CSR instruction reads one of
            // the five registers and writes it, set or cleared or
            // replaced, from a register or a five-bit immediate; ecall
            // and an instruction the core does not know trap; mret
            // returns; ebreak halts.
            let f12 = ir.slice::<20, 12>();
            let is_sys = opcode == 0x73;
            let csr_op = is_sys & (f3 != 0);
            let mip_now = mux(
                sirq,
                mux(tirq, mip | isa::MTIMER, mip) | isa::MSOFT,
                mux(tirq, mip | isa::MTIMER, mip),
            );
            let csr_old = csr_read(
                f12,
                mstatus,
                mtvec,
                self.mscratch.get(),
                mepc,
                self.mcause.get(),
                mie_r,
                mip_now,
                self.mtval.get(),
            );
            let csr_known = csr_known(f12);
            // Whether the instruction writes at all: `csrrw` and
            // `csrrwi` always do, and a set or a clear does only when
            // its source is not `x0` or a zero immediate, which the
            // specification states in terms of the field and not of
            // the value it holds. A write to a read-only register is
            // an illegal instruction; a read of one is not.
            let csr_writes = ((f3 & U::<3>::from(3u8)) == U::<3>::from(1u8))
                | (rs1 != U::<5>::from(0u8));
            let csr_bad = csr_ro(f12) & csr_writes;
            let csr_src = mux(f3.bit(2), rs1.zext::<32>(), a);
            let csr_new = csr_value(f3, csr_old, csr_src);
            let sys0 = is_sys & (f3 == 0);
            let is_ecall = sys0 & (f12 == 0);
            let is_ebreak = sys0 & (f12 == 1);
            let is_mret = sys0 & (f12 == 0x302);
            let known = select!(opcode.raw() => {
                0x37 | 0x17 | 0x6f | 0x67 | 0x63 | 0x03 | 0x23 | 0x13 | 0x33
                | 0x0f => Bit::One,
                0x73 => (csr_op & csr_known & !csr_bad)
                    | is_ecall
                    | is_ebreak
                    | is_mret,
                _ => Bit::Zero,
            });
            // A trap: ecall, a word the core does not know, or the
            // interrupt. The cause, the address and the trap value go to
            // the CSRs, the interrupt enable is saved and cleared, and
            // the handler is the redirect.
            let trap =
                (run & (is_ecall | is_ebreak | !known | unaligned)) | int_take;
            // The exception an unaligned access raises says which way
            // it was going, and its trap value is the address, which is
            // what a handler emulating the access needs.
            let misaligned = mux(
                is_store,
                U::<32>::from(isa::CAUSE_STORE_MISALIGNED),
                U::<32>::from(isa::CAUSE_LOAD_MISALIGNED),
            );
            // The order the specification gives: the external
            // interrupt is taken before the software one, and that
            // before the timer's.
            let cause = mux(
                int_take,
                mux(
                    ext_ok,
                    U::<32>::from(isa::CAUSE_MEXT),
                    mux(
                        soft_ok,
                        U::<32>::from(isa::CAUSE_MSOFT),
                        U::<32>::from(isa::CAUSE_MTIMER),
                    ),
                ),
                mux(
                    is_ecall,
                    U::<32>::from(isa::CAUSE_ECALL),
                    mux(
                        is_ebreak,
                        U::<32>::from(isa::CAUSE_BREAKPOINT),
                        mux(
                            unaligned,
                            misaligned,
                            U::<32>::from(isa::CAUSE_ILLEGAL),
                        ),
                    ),
                ),
            );
            // The trap value: the word for an instruction the core does
            // not know, the address for an unaligned access, and the
            // breakpoint's own address, which is what the
            // specification asks for and what a monitor reads to find
            // out where it stopped.
            let tval = mux(
                run & !known,
                ir,
                mux(unaligned, addr, mux(is_ebreak, pc, U::<32>::from(0u32))),
            );
            let mie_bit = mstatus.bit(3);
            let mpie = mstatus.bit(7);
            let trap_status =
                mux(mie_bit, U::<32>::from(0x80u32), U::<32>::from(0u32));
            let mret_status =
                mux(mpie, U::<32>::from(0x88u32), U::<32>::from(0x80u32));
            let csr_write = run & csr_op & csr_known;
            // Stopped: on a write of an odd value to `mhalt`, and then
            // for good; the halt itself follows a cycle later, when
            // the halting instruction retires. `ebreak` used to do
            // this and now raises the breakpoint exception, so a
            // program that means to stop says so, which is issue 139.
            let halting = csr_write & (f12 == isa::CSR_MHALT) & csr_new.bit(0);
            let stop = mux(run, halting, self.stopped.get());
            let wrote = run & writes & (rd != 0) & !trap;
            let store = run & is_store & !unaligned;
            let wval = select!(opcode.raw() => {
                0x37 => imm_u,
                0x17 => pc + imm_u,
                0x6f | 0x67 => link,
                0x73 => csr_old,
                _ => mux(is_m, m_res, alu),
            });
            // Where the instruction goes next, other than on: a jump's
            // or a taken branch's target, the saved address on mret, the
            // handler on a trap, which is every arm that can trap. The
            // jalr arm is first because it is last to settle: a loaded
            // value forwarded into the add, and this select is on the
            // critical path.
            let target = select!(opcode.raw() => {
                0x67 => (a + imm_i) & !U::<32>::from(1u32),
                0x6f => pc + imm_j,
                0x63 => pc + imm_b,
                0x73 => mux(is_mret, mepc, mtvec),
                _ => mtvec,
            });
            let jump = select!(opcode.raw() => {
                0x6f | 0x67 => Bit::One,
                0x63 => taken,
                0x73 => is_mret | trap,
                _ => trap,
            });

            // The drives. The writeback stage writes the register file
            // and sets the halt. Execute hands the writeback stage what
            // it needs, or nothing. A redirect means the next
            // instruction is not the one the fetch stage read this
            // cycle, so that word is squashed and the fetch restarts at
            // the target; a halt parks the program counter one word
            // past the halting instruction, since the halt retires
            // before the machine stops, as the model's does. The
            // redirect is
            // one mux on the way into the fetch's state, so the target,
            // the last value to settle, passes through as little as
            // possible; the word fetched under a redirect is written
            // and marked empty.
            when!(wb_write => self { regs.at(wb_rd): wb_val });
            self.halted.set(self.halted | self.wb_stop);
            // The bus: a load or a store is a burst of one beat at
            // the address, and a store's beat carries the data with
            // the lanes it covers as its strobe. The load's wait is a
            // register. The identifier is the tracker's to allocate,
            // so the core writes none, and the grant it sends back is
            // of no use here and is dropped.
            let _ = grant.recv_if(grant.peek().is_some());
            let send_store = store & is_dev;
            // A fetch goes out when the words it wants are not in the
            // buffer, nothing else of the core's is out, and the
            // channel has room. The second word is asked for after the
            // first, since only the first says whether it is wanted.
            let f_want = far & (!hit0 | (need1 & !hit1));
            let f_addr = mux(hit0, want + 4, want);
            let f_send = f_want
                & !self.f_wait
                & !self.dev_wait
                & !send_load
                & !send_store
                & issue.ready();
            let send_any = send_load | send_store | f_send;
            if bool::from(send_any) {
                issue.send(Issue {
                    read: !send_store,
                    addr: mux(f_send, f_addr, addr),
                    len: U::<8>::from(0u8),
                    size: U::<3>::from(2u8),
                    burst: BurstKind::Incr,
                    lock: Bit::Zero,
                    cache: U::<4>::from(0u8),
                    prot: U::<3>::from(0u8),
                    qos: U::<4>::from(0u8),
                    region: U::<4>::from(0u8),
                });
            }
            if bool::from(send_store) {
                wbeat.send(W {
                    data: sdata,
                    strb: en,
                    last: Bit::One,
                });
            }
            if bool::from(take_done | take_r) {
                release.send(Grant {
                    id: mux(take_done, dh.id, rh.id),
                });
            }
            // An answer belongs to whichever of the two is out, and
            // only one ever is.
            let f_resp = resp_valid & self.f_wait;
            let d_resp = resp_valid & !self.f_wait;
            let second = self.f_second.get();
            let asked = self.f_asked.get();
            when!(d_resp => self { wb_dev: resp_data });
            with!(self <= {
                f_send ? {
                    f_wait: Bit::One,
                    f_asked: f_addr,
                    f_second: hit0
                },
                f_resp ? f_wait: Bit::Zero,
                f_resp & !second ? {
                    f_w0: resp_data,
                    f_at: asked,
                    f_have: U::<2>::from(1u8)
                },
                f_resp & second ? {
                    f_w1: resp_data,
                    f_have: U::<2>::from(2u8)
                },
                rst ? { f_wait: Bit::Zero, f_have: U::<2>::from(0u8) },
            });
            case!(rst => {
                Bit::One => { self.dev_wait <= Bit::Zero },
                _ if send_load.to_bool() => { self.dev_wait <= Bit::One },
                _ if (self.dev_wait & resp_valid).to_bool() => {
                    self.dev_wait <= Bit::Zero
                },
                _ => {},
            });

            // The CSRs: written by a CSR instruction, by a trap, by mret.
            // The three never coincide in one instruction.
            // The pending bit is set by the line and cleared by
            // software, and the write wins when both fall in one cycle;
            // a trap's writes come after the CSR writes, and win.
            with!(self <= {
                csr_write & (f12 == isa::CSR_MSTATUS) ?
                    mstatus: csr_new & 0x88,
                csr_write & (f12 == isa::CSR_MTVEC) ?
                    mtvec: csr_new & !U::<32>::from(3u32),
                csr_write & (f12 == isa::CSR_MSCRATCH) ? mscratch: csr_new,
                csr_write & (f12 == isa::CSR_MEPC) ?
                    mepc: csr_new & !U::<32>::from(1u32),
                csr_write & (f12 == isa::CSR_MCAUSE) ? mcause: csr_new,
                csr_write & (f12 == isa::CSR_MIE) ? mie: csr_new
                    & U::<32>::from(
                        isa::MEXT | isa::MSOFT | isa::MTIMER,
                    ),
                csr_write & (f12 == isa::CSR_MTVAL) ? mtval: csr_new,
                irq ? mip: mip | isa::MEXT,
                csr_write & (f12 == isa::CSR_MIP) ? mip: csr_new & isa::MEXT,
                trap ? {
                    mepc: pc,
                    mcause: cause,
                    mtval: tval,
                    mstatus: trap_status,
                },
                run & is_mret ? mstatus: mret_status,
            });
            // The sequencer. It starts when an M instruction is in execute
            // with its operands ready, and is released when the
            // instruction runs. A multiply is one step: the product of
            // the magnitudes, from the registers into the pair, which the
            // part's DSP blocks make; a division is thirty-two, each
            // shifting the pair left, subtracting the divisor from the
            // high half when it fits, and shifting the fit in as the
            // quotient bit.
            let m_signed_a = m_signed_a(f3);
            let m_signed_b = m_signed_b(f3);
            let m_neg_a = m_signed_a & a.bit(31);
            let m_neg_b = m_signed_b & b.bit(31);
            let m_abs_a = mux(m_neg_a, U::<32>::from(0u32) - a, a);
            let m_abs_b = mux(m_neg_b, U::<32>::from(0u32) - b, b);
            let m_differ = (m_neg_a & !m_neg_b) | (!m_neg_a & m_neg_b);
            let m_is_div = f3.bit(2);
            // The operands are latched here, so the sequencer does not
            // start while the one in writeback is a device load still
            // waiting for its answer: until the word lands there is
            // nothing to forward, and the register file still holds
            // the value from before the load.
            let m_start =
                m_here & !self.m_busy & !stall_ld & !self.dev_wait & !int_ok;
            let m_step = self.m_busy & !m_done;
            let m_prod = m_lo.zext::<64>().mul::<64>(m_d.zext::<64>());
            let m_t =
                m_hi.slice::<0, 32>().concat::<_, 33>(m_lo.slice::<31, 1>());
            let m_fits = m_t >= m_d.zext::<33>();
            // An interrupt taken while the sequencer runs cancels it:
            // the instruction starts it again when the handler returns,
            // on the registers as they are then, and the sequencer never
            // steps under an instruction other than its own.
            case!(rst => {
                Bit::One => { self.m_busy <= Bit::Zero },
                _ if int_take.to_bool() => { self.m_busy <= Bit::Zero },
                _ if m_start.to_bool() => {
                    self.m_busy <= Bit::One;
                    self.m_count <= mux(m_is_div, U::from(0u8), U::from(31u8));
                    self.m_hi <= 0;
                    self.m_lo <= m_abs_a;
                    self.m_d <= m_abs_b;
                    self.m_neg_q <= mux(
                        m_is_div,
                        m_differ & (b != 0),
                        m_differ
                    );
                    self.m_neg_r <= m_neg_a
                },
                _ if (m_step & !m_is_div).to_bool() => {
                    self.m_count <= self.m_count + 1;
                    self.m_hi <= m_prod.slice::<32, 32>().zext::<33>();
                    self.m_lo <= m_prod.slice::<0, 32>()
                },
                _ if m_step.to_bool() => {
                    self.m_count <= self.m_count + 1;
                    self.m_hi <= mux(m_fits, m_t - m_d.zext::<33>(), m_t);
                    self.m_lo <= m_lo
                        .slice::<0, 31>()
                        .concat::<1, 32>(Bit::from(m_fits).zext())
                },
                _ if (run & is_m).to_bool() => { self.m_busy <= Bit::Zero },
                _ => {},
            });
            // The writeback stage gets the instruction, or the interrupt
            // in its place, which retires as a trap does: nothing written.
            case!(live => {
                Bit::One => {
                    self.wb_valid <= Bit::One;
                    self.wb_pc <= pc;
                    self.wb_ir <= ir;
                    self.wb_rd <= mux(wrote, rd, U::from(0u8));
                    self.wb_alu <= wval;
                    self.wb_f3 <= f3;
                    self.wb_lane <= lane;
                    self.wb_load <= is_load & run;
                    self.wb_stop <= halting
                },
                _ if self.dev_wait.to_bool() => {},
                _ => {
                    self.wb_valid <= Bit::Zero;
                    self.wb_rd <= 0;
                    self.wb_stop <= Bit::Zero
                },
            });
            // begin{fetch}
            // The fetch's next counter: zero on reset, parked one word
            // past the halting instruction, held while halted or
            // stalled, the target on a redirect, else the next
            // instruction, two bytes on or four. Reset, park and
            // hold are known early and the redirect late, so the two
            // candidates fold the early conditions in and the redirect
            // chooses last, one multiplexer from the instruction memory.
            self.redirect.set((run & jump) | int_take);
            let redirect = self.redirect.get();
            let park = run & stop;
            let hold = stall | (stop & !run);
            let zero = U::<32>::from(0u32);
            let width = mux(short, U::<32>::from(2u32), U::<32>::from(4u32));
            let advance = mux(hold, fetch_pc, fetch_pc + width);
            let go = mux(rst, zero, mux(park, link, advance));
            let jmp =
                mux(rst, zero, mux(park, link, mux(int_take, mtvec, target)));
            self.pc.set(mux(redirect, jmp, go));
            case!(rst => {
                Bit::One => { self.valid <= Bit::Zero },
                _ if stall.to_bool() => {},
                _ if stop.to_bool() => { self.valid <= Bit::Zero },
                _ => {
                    self.ir <= fetched;
                    self.ir_c <= Bit::from(short);
                    self.ir_pc <= fetch_pc;
                    self.valid <= !redirect
                },
            });
            // end{fetch}
            self.stopped.set(stop);
            halt.set(self.halted);
            instr.set(mux(wb_here, self.wb_ir.get(), U::<32>::from(0u32)));
            wb.set(Writeback {
                done: wb_here,
                rd: wb_rd,
                val: mux(wb_here, wb_val, U::<32>::from(0u32)),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::expand;
    use crate::isa::compressed;
    use txhdl::types::U;

    /// The core's expander against the decoder's, on every halfword
    /// that is compressed: the same instruction, or, for one that is
    /// none, the halfword itself.
    #[test]
    fn expand_agrees_with_the_decoder() {
        for h in (0u32..0x10000).filter(|h| h & 3 != 3) {
            let want = compressed(h as u16).unwrap_or(h);
            let got = expand(U::<16>::from(h)).raw() as u32;
            assert_eq!(got, want, "{h:#06x}");
        }
    }
}
