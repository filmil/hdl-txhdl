// SPDX-License-Identifier: Apache-2.0
//! Vreteno: a three-stage RV32IMAC core. One process, and on every edge
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
//! stall the pipeline makes by itself. The others wait on something
//! outside it: the multiply and divide sequencer, a fence, the bus, the
//! fetch, a device, and `wfi`. A trap or an `mret` is a redirect like a jump's.
//!
//! Written in the subset `#[lower]` reads: every value is a function
//! of the state and the inputs, `select!` and `mux` choose among
//! values, `with!`, `case!` and `when!` among drives, and the only
//! `if`s are the three that guard a send on the bus. The pieces
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
use txhdl_parts::bus::axi::{BurstKind, Done, Grant, Issue, Resp, R, W};
use txhdl_parts::mmu::{DReq, IReq, Pte, Res};

/// `mstatus`'s fields that user and supervisor mode bring (issue 1012),
/// `MPRV` (issue 1105), and `TVM`, `TW` and `TSR` (issue 1347): what of
/// it is writable, and what `sstatus` shows of it.
const MSTATUS_W: u32 = 0x007e_19aa;
const SSTATUS_W: u32 = 0x000c_0122;
/// The exceptions machine mode may delegate, and the interrupts.
const MEDELEG_W: u32 = 0xb3ff;
const MIDELEG_W: u32 = 0x222;

/// Words of instruction memory. The data memory is a device on the
/// bus, `crate::dmem`, at `DATA_BASE` as the model has it.
pub const IMEM_WORDS: usize = 1024;
/// The boot memory's size in bytes, which is where it ends: a program
/// counter at or above this is fetched from the bus (issue 134).
pub const IMEM_BYTES: u32 = IMEM_WORDS as u32 * 4;
pub const DATA_BASE: u32 = crate::model::DATA_BASE;
/// The data RAM on the core's own port (issue 1275): 64 KiB at
/// `0x1_0000`, the window a program keeps its stack in (issue 1278).
pub const DRAM_BASE: u32 = 0x1_0000;
pub const DRAM_BYTES: u32 = 0x1_0000;

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
/// Whether the instruction cache holds words of a physical page: the
/// data memory's, at 0x1000, or one of DDR3's, from 0x4000_0000 for a
/// gigabyte (issue 1021). The rest are devices and the boot memory.
#[lower]
fn cacheable(page: U<20>) -> Bit {
    Bit::from(page.slice::<18, 2>() == 1) | Bit::from(page == 1)
}

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

/// An AMO's new word, from the old one and the register, by the
/// instruction's five-bit function (issue 1010). It runs in writeback
/// on two registers, off the execute stage's paths.
#[lower]
fn amo_alu(op: U<5>, old: U<32>, b: U<32>) -> U<32> {
    let lt = lt_signed(old, b);
    let ltu = old < b;
    select!(op.raw() => {
        1 => b,
        0 => old + b,
        4 => old ^ b,
        12 => old & b,
        8 => old | b,
        16 => mux(lt, old, b),
        20 => mux(lt, b, old),
        24 => mux(ltu, old, b),
        _ => mux(ltu, b, old),
    })
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

/// The registers a CSR read chooses between, as they stand this cycle.
/// `mip` is the pending register as the core shows it, with the timer's
/// line in it.
#[derive(Value, Clone, Copy, Default, PartialEq, Debug)]
pub struct Csrs {
    pub mcycle: U<64>,
    pub minstret: U<64>,
    pub mstatus: U<32>,
    pub mtvec: U<32>,
    pub mscratch: U<32>,
    pub mepc: U<32>,
    pub mcause: U<32>,
    pub mie: U<32>,
    pub mip: U<32>,
    pub mtval: U<32>,
    pub dcsr: U<32>,
    pub dpc: U<32>,
    pub busquiet: Bit,
    /// User and supervisor mode (issue 1012).
    pub medeleg: U<32>,
    pub mideleg: U<32>,
    pub counteren: U<6>,
    pub stvec: U<32>,
    pub sscratch: U<32>,
    pub menv_fiom: Bit,
    pub senv_fiom: Bit,
    pub sepc: U<32>,
    pub scause: U<32>,
    pub stval: U<32>,
    pub satp: U<32>,
    pub time: U<64>,
    /// `mhartid`, the hart's number (issue 1408).
    pub hartid: U<32>,
}

/// Which of the CSRs that read as anything but zero a number names:
/// one to thirty-eight, in the order [`csr_read_at`] reads them, and
/// zero for every other number, which reads as zero. The core finds it
/// from the instruction register, into a register of its own, so that
/// the read is a select on a register rather than the number's decode
/// followed by the select (issue 1260); a CSR instruction waits a cycle
/// for it (issue 1295).
#[lower]
fn csr_index(f12: U<12>) -> U<6> {
    select!(f12.raw() => {
        0x300 => U::<6>::from(1u8),
        0x305 => U::<6>::from(2u8),
        0x340 => U::<6>::from(3u8),
        0x341 => U::<6>::from(4u8),
        0x342 => U::<6>::from(5u8),
        0x7b0 => U::<6>::from(6u8),
        0x7b1 => U::<6>::from(7u8),
        0x7c1 => U::<6>::from(8u8),
        0x304 => U::<6>::from(9u8),
        0x344 => U::<6>::from(10u8),
        0x343 => U::<6>::from(11u8),
        0x301 => U::<6>::from(12u8),
        0xb00 => U::<6>::from(13u8),
        0xb80 => U::<6>::from(14u8),
        0xb02 => U::<6>::from(15u8),
        0xb82 => U::<6>::from(16u8),
        0x302 => U::<6>::from(17u8),
        0x303 => U::<6>::from(18u8),
        0x306 => U::<6>::from(19u8),
        0x106 => U::<6>::from(20u8),
        0x100 => U::<6>::from(21u8),
        0x104 => U::<6>::from(22u8),
        0x144 => U::<6>::from(23u8),
        0x105 => U::<6>::from(24u8),
        0x140 => U::<6>::from(25u8),
        0x141 => U::<6>::from(26u8),
        0x142 => U::<6>::from(27u8),
        0x143 => U::<6>::from(28u8),
        0x180 => U::<6>::from(29u8),
        0xc00 => U::<6>::from(30u8),
        0xc01 => U::<6>::from(31u8),
        0xc81 => U::<6>::from(32u8),
        0xc80 => U::<6>::from(33u8),
        0xc02 => U::<6>::from(34u8),
        0xc82 => U::<6>::from(35u8),
        0x30a => U::<6>::from(36u8),
        0x10a => U::<6>::from(37u8),
        0xf14 => U::<6>::from(38u8),
        _ => U::<6>::from(0u8),
    })
}

/// A CSR read, by the index [`csr_index`] gives its number (issue
/// 1260). The registers come as one struct, which the lowering reads a
/// field of at a time (issue 504).
#[lower]
fn csr_read_at(at: U<6>, c: Csrs) -> U<32> {
    select!(at.raw() => {
        1 => c.mstatus,
        2 => c.mtvec,
        3 => c.mscratch,
        4 => c.mepc,
        5 => c.mcause,
        6 => c.dcsr,
        7 => c.dpc,
        8 => c.busquiet.zext::<32>(),
        9 => c.mie,
        10 => c.mip,
        11 => c.mtval,
        12 => U::<32>::from(isa::MISA),
        13 => c.mcycle.slice::<0, 32>(),
        14 => c.mcycle.slice::<32, 32>(),
        15 => c.minstret.slice::<0, 32>(),
        16 => c.minstret.slice::<32, 32>(),
        17 => c.medeleg,
        18 => c.mideleg,
        19 => c.counteren.slice::<0, 3>().zext::<32>(),
        20 => c.counteren.slice::<3, 3>().zext::<32>(),
        21 => c.mstatus & U::<32>::from(SSTATUS_W),
        22 => c.mie & c.mideleg,
        23 => c.mip & c.mideleg,
        24 => c.stvec,
        25 => c.sscratch,
        26 => c.sepc,
        27 => c.scause,
        28 => c.stval,
        29 => c.satp,
        30 => c.mcycle.slice::<0, 32>(),
        31 => c.time.slice::<0, 32>(),
        32 => c.time.slice::<32, 32>(),
        33 => c.mcycle.slice::<32, 32>(),
        34 => c.minstret.slice::<0, 32>(),
        35 => c.minstret.slice::<32, 32>(),
        36 => c.menv_fiom.zext::<32>(),
        37 => c.senv_fiom.zext::<32>(),
        38 => c.hartid,
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
        | 0x343 | 0x7c0 | 0x7c1 | 0x7b0 | 0x7b1 | 0x301 | 0xf11 | 0xf12 | 0xf13
        | 0xf14 | 0xb00 | 0xb02 | 0xb80 | 0xb82 | 0x302 | 0x303 | 0x306 | 0x310
        | 0x100 | 0x104 | 0x105 | 0x106 | 0x140 | 0x141 | 0x142 | 0x143
        | 0x144 | 0x180 | 0xc00 | 0xc01 | 0xc02 | 0xc80 | 0xc81
        | 0xc82 | 0x30a | 0x31a | 0x10a => Bit::One,
        // The counters that count nothing here, which the privileged
        // specification has a hart hold all the same, each reading as
        // zero (issue 1460): `mcountinhibit`, `mhpmevent3` to `31`,
        // `mhpmcounter3` to `31` and their upper halves, and the
        // user's copies.
        0x320 | 0x323..=0x33f | 0xb03..=0xb1f | 0xb83..=0xb9f
        | 0xc03..=0xc1f | 0xc83..=0xc9f => Bit::One,
        _ => Bit::Zero,
    })
}

/// Whether a CSR number is one of the read-only ones, whose address
/// begins with two set bits. A write to one is an illegal
/// instruction; a read is not.
#[lower]
fn csr_ro(f12: U<12>) -> Bit {
    Bit::from(f12.slice::<10, 2>() == 3)
}

/// One line's bit in a half of the data cache's valid bits, or none
/// (issue 1275).
#[lower]
fn dc_bit(line: U<8>, en: Bit, hi: Bit) -> U<128> {
    let here = en & mux(hi, line.bit(7), !line.bit(7));
    // Made from one bit widened, so that the netlist writes no literal
    // of 128 bits.
    let one = mux(here, U::<1>::from(1u8), U::<1>::from(0u8));
    one.zext::<128>() << (line.slice::<0, 7>().raw() as usize)
}

/// Line `line`'s valid bit, of the two halves (issue 1275).
#[lower]
fn dc_vget(v0: U<128>, v1: U<128>, line: U<8>) -> Bit {
    let v = mux(line.bit(7), v1, v0);
    (v >> (line.slice::<0, 7>().raw() as usize)).bit(0)
}

/// The bits of a word a store's four lanes cover.
#[lower]
fn lane_mask(en: U<4>) -> U<32> {
    mux(
        en.bit(3),
        U::<32>::from(0xff00_0000u32),
        U::<32>::from(0u32),
    ) | mux(
        en.bit(2),
        U::<32>::from(0x00ff_0000u32),
        U::<32>::from(0u32),
    ) | mux(
        en.bit(1),
        U::<32>::from(0x0000_ff00u32),
        U::<32>::from(0u32),
    ) | mux(
        en.bit(0),
        U::<32>::from(0x0000_00ffu32),
        U::<32>::from(0u32),
    )
}

/// Bit `i` of a word, for a delegation register read by a cause
/// (issue 1012).
#[lower]
fn bit_of(v: U<32>, i: U<5>) -> Bit {
    (v >> (i.raw() as usize)).bit(0)
}

/// The interrupt taken of a set of pending and enabled ones, in the
/// specification's order: external, software, timer, the machine's
/// before the supervisor's (issue 1012).
#[lower]
fn int_cause(set: U<32>) -> U<32> {
    mux(
        set.bit(11),
        U::<32>::from(isa::CAUSE_MEXT),
        mux(
            set.bit(3),
            U::<32>::from(isa::CAUSE_MSOFT),
            mux(
                set.bit(7),
                U::<32>::from(isa::CAUSE_MTIMER),
                mux(
                    set.bit(9),
                    U::<32>::from(isa::CAUSE_SEXT),
                    mux(
                        set.bit(1),
                        U::<32>::from(isa::CAUSE_SSOFT),
                        U::<32>::from(isa::CAUSE_STIMER),
                    ),
                ),
            ),
        ),
    )
}

/// What a write leaves of `mstatus`: its writable fields, with `MPP`
/// one of the three modes there are, a 2 taken as user mode.
#[lower]
fn mstatus_w(v: U<32>) -> U<32> {
    let v = v & U::<32>::from(MSTATUS_W);
    mux(v.slice::<11, 2>() == 2, v & !U::<32>::from(0x1800u32), v)
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
#[lower]
fn m_result(f3: U<3>, hi: U<33>, lo: U<32>, neg_q: Bit, neg_r: Bit) -> U<32> {
    let mag = hi.slice::<0, 32>().concat::<_, 64>(lo);
    let p = mux(neg_q, U::<64>::from(0u32) - mag, mag);
    let q = mux(neg_q, U::<32>::from(0u32) - lo, lo);
    let rem = hi.slice::<0, 32>();
    let r = mux(neg_r, U::<32>::from(0u32) - rem, rem);
    select!(f3.raw() => {
        0 => p.slice::<0, 32>(),
        1..=3 => p.slice::<32, 32>(),
        4..=5 => q,
        _ => r,
    })
}

/// The data memory is not here: it is a device on the bus, `dmem.rs`,
/// as every address is (issue 268).
///
/// The state. The fetch stage's program counter. The instruction
/// register, its program counter and whether it holds an instruction,
/// the boundary between fetch and execute. The writeback registers,
/// the boundary between execute and writeback: whether one is there,
/// its destination, its value if not a load, the raw word read for a
/// load with the lane and the width to take from it, and whether it
/// halts. The halt itself, which the writeback stage sets. And the two
/// memories, the registers and the boot memory.
#[derive(Trace, Default)]
pub struct Vreteno<
    const IW: usize,
    const DW: usize = 16384,
    const IC: usize = 1,
    const HID: usize = 0,
    const EXCL: usize = 0,
> {
    pub pc: Reg<U<32>>,
    pub ir: Reg<U<32>>,
    pub ir_c: Reg<Bit>,
    /// The word in execute came from a refused fetch: it is zero, and
    /// its trap is the instruction access fault (issue 423).
    pub ir_bad: Reg<Bit>,
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
    // begin{debug}
    /// Debug mode (issue 154): the core is halted by a debugger and
    /// resumable, where `halted` is a program's own end. Entered on a
    /// halt request, on the instruction after a single step, or on an
    /// `ebreak` when `dcsr.ebreakm` says so; left on a resume request,
    /// to `dpc`. No instruction executes and no interrupt is taken
    /// while in it.
    pub debug: Reg<Bit>,
    /// The instruction to execute on resume: the one not executed on
    /// entry.
    pub dpc: Reg<U<32>>,
    /// `dcsr` as read: the version, `ebreakm`, the cause, `step`, the
    /// privilege.
    pub dcsr: Reg<U<32>>,
    /// Entry this cycle, as a wire: the instruction in execute is not
    /// run and no interrupt is taken in its place.
    pub dbg_take: Wire<Bit>,
    /// A single step in progress: armed on a resume with `dcsr.step`
    /// set, and once one instruction has run, `stepped`, which is a
    /// request to enter again before the next.
    pub step_armed: Reg<Bit>,
    pub stepped: Reg<Bit>,
    /// The resume request seen: a level acted on once per entry.
    pub resume_seen: Reg<Bit>,
    // end{debug}
    pub mstatus: Reg<U<32>>,
    pub mtvec: Reg<U<32>>,
    pub mscratch: Reg<U<32>>,
    pub mepc: Reg<U<32>>,
    pub mcause: Reg<U<32>>,
    pub mie: Reg<U<32>>,
    pub mip: Reg<U<32>>,
    pub mtval: Reg<U<32>>,
    /// User and supervisor mode (issue 1012): the privilege, 3 machine,
    /// 1 supervisor, 0 user; the delegations; the supervisor's pending
    /// bits software sets; its registers; and the counter enables,
    /// machine's low and supervisor's high.
    pub prv: Reg<U<2>>,
    pub medeleg: Reg<U<32>>,
    pub mideleg: Reg<U<32>>,
    pub mip_sw: Reg<U<32>>,
    pub stvec: Reg<U<32>>,
    pub sscratch: Reg<U<32>>,
    /// `menvcfg.FIOM` and `senvcfg.FIOM`, the one field of each the core
    /// keeps (issue 1348). They change nothing: a `fence` here already
    /// waits for every store, whatever it was to.
    pub menv_fiom: Reg<Bit>,
    pub senv_fiom: Reg<Bit>,
    pub sepc: Reg<U<32>>,
    pub scause: Reg<U<32>>,
    pub stval: Reg<U<32>>,
    pub satp: Reg<U<32>>,
    pub counteren: Reg<U<6>>,
    pub wb_dev: Reg<U<32>>,
    /// Whether the bus refused the load whose answer is in `wb_dev`:
    /// the load then traps as it retires, a load access fault at its
    /// address, and writes nothing (issue 417).
    pub wb_err: Reg<Bit>,
    /// The data RAM on the core's own port (issue 1275): 64 KiB at
    /// DRAM_BASE, the window a program keeps its stack in, answered in
    /// the cycle after a load runs rather than over the bus. Four lanes
    /// of a byte, so a store's strobes are four write enables; each is
    /// written at one address and read at one, a block RAM. Nothing but
    /// the core reaches it. DW words a lane: 16384 on the board, and a
    /// few in the netlist the documents and the layout take, where the
    /// window's addresses wrap.
    pub dl0: Mem<U<8>, DW>,
    pub dl1: Mem<U<8>, DW>,
    pub dl2: Mem<U<8>, DW>,
    pub dl3: Mem<U<8>, DW>,
    /// The word the lanes read, at the address the access in execute
    /// had, a cycle ago; and whether the load in writeback is one of
    /// theirs.
    pub dl_word: Reg<U<32>>,
    pub wb_loc: Reg<Bit>,
    /// The data cache for the DDR3 (issue 1275): 4 KiB, 256 lines of
    /// four words, direct mapped, indexed by the page offset's bits 11
    /// to 4 and tagged by the physical page, written through, and
    /// filled by a load's miss alone. The words and the tags are block
    /// RAMs read into registers at the address of the access in
    /// execute; the valid bits are registers, so that another host's
    /// write, the core's own invalidations and a fill can all change
    /// them in one cycle.
    pub dc_data: Mem<U<32>, 1024>,
    pub dc_tag: Mem<U<20>, 256>,
    pub dc_v0: Reg<U<128>>,
    pub dc_v1: Reg<U<128>>,
    pub dc_word: Reg<U<32>>,
    pub dc_tagr: Reg<U<20>>,
    pub dc_vr: Reg<Bit>,
    /// What the cache took in the cycle before, which execute's reads
    /// in that cycle did not see (issue 1431): the word written and its
    /// address; a line kept and its tag; the lines taken out by another
    /// host's write, by an AMO, lr.w or sc.w, and by a miss or a fill
    /// not kept; and whether all were.
    pub dc_fd_v: Reg<Bit>,
    pub dc_fd_a: Reg<U<10>>,
    pub dc_fd: Reg<U<32>>,
    pub dc_fk_v: Reg<Bit>,
    pub dc_fk_line: Reg<U<8>>,
    pub dc_fk_tag: Reg<U<20>>,
    pub dc_fs_v: Reg<Bit>,
    pub dc_fs_line: Reg<U<8>>,
    pub dc_fo_v: Reg<Bit>,
    pub dc_fo_line: Reg<U<8>>,
    pub dc_fm_v: Reg<Bit>,
    pub dc_fm_line: Reg<U<8>>,
    pub dc_fall: Reg<Bit>,
    /// Whether the load in writeback may be the cache's; where its miss
    /// is: 0 the lookup, 1 waiting to ask, 2 the answer's beats coming;
    /// the next beat; whether the fill is not to be kept; and whether
    /// it is a single word read around the cache.
    pub wb_dc: Reg<Bit>,
    pub dc_st: Reg<U<2>>,
    pub dc_beat: Reg<U<2>>,
    pub dc_kill: Reg<Bit>,
    pub dc_one: Reg<Bit>,
    /// A store in writeback that may update a line the cache holds: its
    /// data and its lanes.
    pub wb_st: Reg<Bit>,
    pub wb_sd: Reg<U<32>>,
    pub wb_en: Reg<U<4>>,
    /// A store the bus refused, kept until the trap for it is taken
    /// before the next instruction to run: a store is posted, so its
    /// fault is raised late and without the address (issue 417).
    pub st_err: Reg<Bit>,
    /// Stores posted and not yet answered, which is what a `fence`
    /// waits for: the tracker gives out four identifiers, so at most
    /// four are out (issue 432).
    pub stores_out: Reg<U<3>>,
    /// The A extension (issue 1010). The reservation `lr.w` makes: a
    /// word's address, and whether it holds.
    pub rsv_valid: Reg<Bit>,
    pub rsv_at: Reg<U<30>>,
    /// The reservation compare of last cycle, and whether it was made
    /// for the `sc.w` in execute with its operand valid: an `sc.w`
    /// waits a cycle and uses the registered compare, which keeps the
    /// compare off every store's issue (issue 1298).
    pub sc_q: Reg<Bit>,
    pub sc_ready: Reg<Bit>,
    /// An AMO in writeback: whether the instruction there is one, its
    /// function and its register operand, kept from execute; its
    /// phase after the load's answer, 1 to compute, 2 to store, and for
    /// an exclusive pair on the DDR3 3 to wait for the store's answer
    /// and 4 to read again when it did not store (issue 1408); and the
    /// word it stores.
    pub wb_amo: Reg<Bit>,
    pub amo_op: Reg<U<5>>,
    pub amo_b: Reg<U<32>>,
    pub amo_ph: Reg<U<3>>,
    /// The AMO in writeback is an exclusive pair on the DDR3; and the
    /// `sc.w` there is an exclusive write on the DDR3, whether its
    /// answer is still to come, and whether it said the store failed
    /// (issue 1408).
    pub wb_x: Reg<Bit>,
    pub wb_scx: Reg<Bit>,
    pub scx_wait: Reg<Bit>,
    pub scx_fail: Reg<Bit>,
    pub amo_val: Reg<U<32>>,
    /// `mbusquiet`: bus refusals read zero and drop the store instead
    /// of trapping, for a program that polls a peripheral which may
    /// refuse on purpose (issue 417).
    pub busquiet: Reg<Bit>,
    pub dev_wait: Reg<Bit>,
    /// The two machine counters. `mcycle` counts every cycle the core
    /// is running and `minstret` every instruction it retires, so the
    /// difference between them is what the pipeline spent waiting
    /// (issue 299).
    pub mcycle: Reg<U<64>>,
    pub minstret: Reg<U<64>>,
    /// Waiting for an interrupt, which `wfi` asks for: the core
    /// fetches nothing until one is pending and enabled (issue 275).
    pub waiting: Reg<Bit>,
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
    /// The register the first read port reads: the instruction's `rs1`,
    /// or the debug module's number in debug mode, chosen a cycle
    /// early so that the read starts at a register and not behind a
    /// multiplexer (issue 1130).
    pub ra_at: Reg<U<5>>,
    /// Whether the register the writeback stage writes is the one each
    /// operand reads, and not x0: the forwarding's two compares, made a
    /// cycle early so that the operand waits on no compare (issues 1130
    /// and 1288).
    pub m_a: Reg<Bit>,
    pub m_b: Reg<Bit>,
    /// Which CSR the instruction reads, as [`csr_index`] gives it: found from
    /// the instruction register's number, or the debug module's in debug
    /// mode, so that the read starts at a register (issue 1260). It lags
    /// the instruction register by a cycle, which a CSR instruction waits
    /// out (issue 1295).
    pub csr_at: Reg<U<6>>,
    /// The instruction register was loaded at the last edge: the
    /// instruction in execute is in its first cycle, before `csr_at`
    /// has caught up with it (issue 1295).
    pub ir_new: Reg<Bit>,
    /// An exception the instruction in execute raised last cycle, and
    /// the handler it goes to: the fetch is redirected there a cycle
    /// after the trap, so that the decision, which settles behind the
    /// operands and the address's adder, reaches only registers and
    /// not the next program counter (issue 1195).
    pub tp: Reg<Bit>,
    pub tp_vec: Reg<U<32>>,
    /// Whether an interrupt is pending and enabled, as the state
    /// stood last cycle; whether it is one machine mode keeps; and the
    /// pending set it was chosen from. The take reads these and not
    /// the enables' logic, which was the flagship's worst path into
    /// the fetch's address (issue 1331).
    pub int_q: Reg<Bit>,
    pub int_m_q: Reg<Bit>,
    pub int_set_q: Reg<U<32>>,
    /// A conditional branch was guessed wrong last cycle, and where it
    /// really goes: a branch is guessed taken when it points back and
    /// not taken when it points forward, from the instruction alone,
    /// and its compare, registered, corrects a wrong guess a cycle
    /// later, as an exception redirects (issue 1300).
    pub bp_fix: Reg<Bit>,
    pub bp_vec: Reg<U<32>>,
    pub imem: Mem<U<32>, IMEM_WORDS>,
    /// A fetch that is out on the bus, for a program above the boot
    /// memory: whether one is out, the word it asked for, the two
    /// words it has brought back, how many of them, and the address
    /// they start at.
    pub f_wait: Reg<Bit>,
    pub f_asked: Reg<U<32>>,
    pub f_w0: Reg<U<32>>,
    pub f_w1: Reg<U<32>>,
    /// Whether the bus refused each word: an instruction taken from a
    /// refused word is the instruction access fault (issue 423).
    pub f_bad0: Reg<Bit>,
    pub f_bad1: Reg<Bit>,
    pub f_have: Reg<U<2>>,
    pub f_at: Reg<U<32>>,
    /// The address of the buffer's second word, `f_at` plus four, kept
    /// in a register so that the window's second compare has no add in
    /// front of it (issue 1187).
    pub f_at4: Reg<U<32>>,
    /// The buffer is to shift when the word out for its second place
    /// comes back: the counter moved on into that word while it was
    /// out, so it goes into the first place instead (issue 1279).
    pub f_sh: Reg<Bit>,
    /// Whether the fetch that is out is for the second word of a
    /// thirty-two bit instruction that straddles two words.
    pub f_second: Reg<Bit>,
    // begin{icache}
    /// The instruction cache (issue 1021): sixteen kilobytes (issue
    /// 1290), one way, 1024 lines of four words. A fetch goes to it
    /// once translated, so it is indexed and tagged by the physical
    /// address: two virtual pages of one physical page share its lines.
    /// It holds words of the data memory and of DDR3, the two memories
    /// behind the bus.
    ///
    /// The words are in two banks, the even and the odd (issue 1303):
    /// a lookup reads a word and the one after it, which are always in
    /// different banks, so each bank is read once a cycle and written
    /// by the fill, and is a block RAM.
    pub ic_even: Mem<U<32>, 2048>,
    pub ic_odd: Mem<U<32>, 2048>,
    /// Each line's tag: bit 20 says the line is valid, and the low
    /// twenty bits are its physical page, of which the index holds the
    /// low two as well.
    pub ic_tag: Mem<U<21>, 1024>,
    /// Where a cached fetch is: 0 not in the cache, 1 looking up
    /// `ic_pa` in it, 2 filling its line from the bus, 3 reading the
    /// filled line's words before looking it up again.
    pub ic_st: Reg<U<2>>,
    /// The physical address of the word the cached fetch wants.
    pub ic_pa: Reg<U<32>>,
    /// Each bank's word for the lookup, read the cycle before it, so
    /// that the read is a block RAM's own registered read (issue
    /// 1303): when the fetch goes to the cache, and again once a fill
    /// has written the line.
    pub ic_ewd: Reg<U<32>>,
    pub ic_owd: Reg<U<32>>,
    /// The line's tag for the lookup, read the same way (issue 1309);
    /// and whether the tags were being cleared a cycle ago, since a
    /// lookup that waited on the clearing reads its tag again the cycle
    /// after the last line is cleared, and looks it up the cycle after.
    pub ic_tdat: Reg<U<21>>,
    pub ic_clr2: Reg<Bit>,
    /// The next beat of a fill, and whether one of them was refused.
    pub ic_beat: Reg<U<2>>,
    pub ic_bad: Reg<Bit>,
    /// The tags being cleared after a reset or a `fence.i`, a line a
    /// cycle, and the next line to clear.
    pub ic_clearing: Reg<Bit>,
    pub ic_clr: Reg<U<10>>,
    // end{icache}
    // begin{vm}
    /// Virtual memory (issue 1014). The fetch's translation of one
    /// page, the last it asked for: whether there is one, the virtual
    /// page, the physical page, and whether the page is a page fault
    /// or an access fault, which the fetch then takes its words as.
    pub ft_valid: Reg<Bit>,
    pub ft_vpn: Reg<U<20>>,
    pub ft_ppn: Reg<U<20>>,
    pub ft_pf: Reg<Bit>,
    pub ft_af: Reg<Bit>,
    /// The fetch's request to the unit: whether one is out, the
    /// address, and how many cycles it has been held, up to two.
    pub i_req: Reg<Bit>,
    pub i_va: Reg<U<32>>,
    pub i_age: Reg<U<2>>,
    /// Whether each word in the fetch buffer is a page fault.
    pub f_pf0: Reg<Bit>,
    pub f_pf1: Reg<Bit>,
    /// The fetch out on the bus was asked under translations that
    /// have since gone, so its answer is dropped.
    pub f_drop: Reg<Bit>,
    /// The instruction in execute is a fetch's page fault, and the
    /// fault is in its second half, which is on the next page.
    pub ir_pf: Reg<Bit>,
    pub ir_pf2: Reg<Bit>,
    /// The data's request to the unit, for the load, store or AMO in
    /// execute: whether one is out, the address, how many cycles it
    /// has been held, and whether it is for a store.
    pub x_req: Reg<Bit>,
    pub x_va: Reg<U<32>>,
    pub x_age: Reg<U<2>>,
    pub x_st: Reg<Bit>,
    /// The answer, kept until the instruction leaves execute: whether
    /// there is one, the physical address, and the two faults.
    pub x_done: Reg<Bit>,
    pub x_pa: Reg<U<32>>,
    pub x_pf: Reg<Bit>,
    pub x_af: Reg<Bit>,
    /// The flush the unit is told of, the cycle after an
    /// `sfence.vma` or a write of `satp` ran.
    pub flush: Reg<Bit>,
    /// A read of the walker's is out on the bus.
    pub p_wait: Reg<Bit>,
    /// The physical address of the access in writeback, which an
    /// AMO's store goes to.
    pub wb_pa: Reg<U<32>>,
    // end{vm}
}

impl<
        const IW: usize,
        const DW: usize,
        const IC: usize,
        const HID: usize,
        const EXCL: usize,
    > Vreteno<IW, DW, IC, HID, EXCL>
{
    /// The data RAM's depth is a power of two, since its addresses are
    /// masked to it with `DW - 1` (issue 1275).
    const DW_POW2: () = assert!(
        DW.is_power_of_two(),
        "the data RAM's depth is a power of two"
    );

    /// A core with its program loaded.
    pub fn with(program: &[u32]) -> Self {
        let () = Self::DW_POW2;
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
        if self.debug.get().to_bool() {
            self.dpc.get()
        } else if self.wb_valid.get().to_bool() {
            self.wb_pc.get()
        } else if self.valid.get().to_bool() {
            self.ir_pc.get()
        } else {
            self.pc.get()
        }
    }
}

#[lower]
impl<
        const IW: usize,
        const DW: usize,
        const IC: usize,
        const HID: usize,
        const EXCL: usize,
    > Unit for Vreteno<IW, DW, IC, HID, EXCL>
{
    async fn run(
        &mut self,
        (
            rst,
            irq,
            tirq,
            sirq,
            rdata,
            done,
            grant,
            haltreq,
            resumereq,
            dbg_regno,
            dbg_wdata,
            dbg_we,
            time,
            seirq,
            dc_snoop,
            ires,
            dres,
            ptw,
        ): (
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
            Rx<R<32, IW>>,
            Rx<Done<IW>>,
            Rx<Grant<IW>>,
            In<Bit>,
            In<Bit>,
            In<U<16>>,
            In<U<32>>,
            In<Bit>,
            // The timer's count, which `time` reads (issue 1012).
            In<U<64>>,
            // The interrupt controller's supervisor line, `mip.SEIP`'s
            // (issue 1094).
            In<Bit>,
            // Another host's write into the DDR3, answered: the data
            // cache's line to invalidate, bit 8 its valid (issue 1275).
            In<U<9>>,
            // The memory management unit's answers, registers, to the
            // fetch and to the data, and its walker's reads (issues
            // 1014 and 1122).
            In<Res>,
            In<Res>,
            Rx<U<32>>,
        ),
        (
            halt,
            instr,
            wb,
            issue,
            wbeat,
            release,
            dbg,
            dbg_rdata,
            asleep,
            mmu_satp,
            mmu_prv,
            mmu_sum,
            mmu_mxr,
            mmu_flush,
            ireq,
            dreq,
            pte,
        ): (
            Out<Bit>,
            Out<U<32>>,
            Out<Writeback>,
            Tx<Issue<32>>,
            Tx<W<32, 4>>,
            Tx<Grant<IW>>,
            Out<Bit>,
            Out<U<32>>,
            // Whether the core waits in `wfi` (issue 1408).
            Out<Bit>,
            // What the unit translates by, the requests, and the
            // walker's answers (issue 1014).
            Out<U<32>>,
            Out<U<2>>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<IReq>,
            Out<DReq>,
            Tx<Pte>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let (rst, irq, tirq) = (rst.get(), irq.get(), tirq.get());
            let sirq = sirq.get();
            // The debugger's two requests, and the debug state.
            let (haltreq, resumereq) = (haltreq.get(), resumereq.get());
            // The debug module's register access, honoured in debug mode
            // only: a number in the specification's space, 0x1000 and
            // up a general register, below that a CSR, the word to
            // write and whether to. The register file's first read port
            // and its write port are lent to it, since nothing runs.
            let (dbg_regno, dbg_wdata, dbg_we) =
                (dbg_regno.get(), dbg_wdata.get(), dbg_we.get());
            let dbg_gpr = dbg_regno.slice::<0, 5>();
            let dbg_is_gpr = dbg_regno.bit(12);
            let dbg_csr = dbg_regno.slice::<0, 12>();
            let in_debug = self.debug.get();
            let (dcsr, dpc) = (self.dcsr.get(), self.dpc.get());
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
            // Whether the answer is a refusal: the peripheral failed, or
            // there is none at the address and the router said so.
            let resp_bad = select!(rh.resp => {
                Resp::Okay => Bit::Zero,
                Resp::ExOkay => Bit::Zero,
                _ => Bit::One,
            });
            let done_bad = select!(dh.resp => {
                Resp::Okay => Bit::Zero,
                Resp::ExOkay => Bit::Zero,
                _ => Bit::One,
            });
            let (m_hi, m_lo, m_d) =
                (self.m_hi.get(), self.m_lo.get(), self.m_d.get());
            // The writeback stage: a loaded word's byte or half, by the
            // lane and the width the execute stage read with it,
            // extended; else the value execute computed. Written to the
            // register file, and forwarded to execute below.
            // A load's word: what the bus answered, in its register.
            // From the data RAM's lanes, the cycle after the load ran
            // (issue 1275); an AMO's word is the one it captured.
            let loaded = extended(
                self.wb_f3.get(),
                self.wb_lane.get(),
                mux(
                    self.wb_loc & !self.wb_amo,
                    self.dl_word.get(),
                    self.wb_dev.get(),
                ),
            );
            // An exclusive `sc.w`'s result is its answer's (issue 1408).
            let wb_val = mux(
                self.wb_scx,
                self.scx_fail.get().zext::<32>(),
                mux(self.wb_load, loaded, wb_alu),
            );
            // A device load sits in writeback while its wait is on, and
            // retires the cycle after its answer has landed.
            // An AMO sits in writeback after its load's answer, while
            // its word is computed and stored (issue 1010).
            let amo_ph = self.amo_ph.get();
            let amo_busy = amo_ph != 0;
            let scx_wait = self.scx_wait.get();
            let wb_here =
                self.wb_valid & !self.dev_wait & !amo_busy & !scx_wait;
            // A refused load retires as a trap: nothing is written, the
            // instruction behind it is squashed, and the fetch restarts
            // at the handler. Its address is what the ALU computed.
            let wb_fault = wb_here
                & self.wb_load
                & self.wb_err
                & !self.busquiet
                & !self.wb_scx;
            let wb_write = wb_here & (wb_rd != 0) & !wb_fault;
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
            //
            // Virtual memory (issue 1014): translation is on below
            // machine mode when `satp` says, for the fetch and the data
            // alike, and then every fetch is from the bus, at the
            // address the unit translated, and the buffer holds words
            // by their virtual address.
            let satp_r = self.satp.get();
            let vm = satp_r.bit(31) & Bit::from(self.prv.get() != 3);
            // A load or a store in machine mode with `MPRV` set is
            // translated and checked as the mode `MPP` names, which is
            // how OpenSBI reads a supervisor's memory; the fetch is not
            // (issue 1105).
            let mprv = mstatus.bit(17) & Bit::from(self.prv.get() == 3);
            let dprv = mux(mprv, mstatus.slice::<11, 2>(), self.prv.get());
            let dvm = satp_r.bit(31) & Bit::from(dprv != 3);
            let far = Bit::from(fetch_pc >= U::<32>::from(IMEM_BYTES)) | vm;
            let want = fetch_pc & U::<32>::from(0xffff_fffcu32);
            let f_at = self.f_at.get();
            let f_have = self.f_have.get();
            let hit0 = (f_have != 0) & (f_at == want);
            let hit1 = (f_have == 2) & (f_at == want);
            // The buffer holds two words (issue 1187), and an
            // instruction is read from the first. When the next one in a
            // straight run begins in the second, the buffer shifts along
            // at the edge, so the read and the fetch compare only the
            // first word's address, a register, with the counter
            // (issue 1279).
            let f_at4 = self.f_at4.get();
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
            // file, which has it by then. The stall is the one wait the
            // pipeline makes by itself; the others, ORed into `stall`
            // below, wait on something outside it.
            //
            // The select waits on registers alone (issue 1288): `m_a` and
            // `m_b` already say the number is the one written and not x0,
            // and a refused load, `wb_fault`, need not be ruled out, since
            // the instruction in execute is then not live and is squashed,
            // so what it would have been given is never used. The same
            // holds of the waits in writeback, a device load's, an AMO's
            // and an exclusive sc.w's: each is ORed into the stall, so the
            // instruction in execute is not live while one stands, and the
            // cycle it ends the word forwarded is the right one. Kept out
            // of the select, they are off the path through the ALU into
            // the fetch, where the sc.w's wait was the flagship's worst
            // (issue 1549).
            let fwd_a = self.wb_valid & self.m_a;
            let fwd_b = self.wb_valid & self.m_b;
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
            //
            // With user and supervisor mode (issue 1012) an interrupt
            // machine mode keeps is taken below machine mode always and
            // in it with `MIE`; one it delegated in `mideleg` is taken
            // below supervisor mode always, in it with `SIE`, and never
            // in machine mode; and the machine's come first.
            let mtip = tirq;
            let prv = self.prv.get();
            let mideleg = self.mideleg.get();
            let mip_all = mip
                | mux(mtip, U::<32>::from(isa::MTIMER), U::<32>::from(0u32))
                | mux(sirq, U::<32>::from(isa::MSOFT), U::<32>::from(0u32))
                | self.mip_sw.get();
            let pend = mip_all & mie_r;
            let m_on = (prv != 3) | mstatus.bit(3);
            let s_on = (prv == 0) | ((prv == 1) & mstatus.bit(1));
            let m_set = mux(m_on, pend & !mideleg, U::<32>::from(0u32));
            let s_set = mux(s_on, pend & mideleg, U::<32>::from(0u32));
            let int_set = mux(m_set != 0, m_set, s_set);
            // The decision is acted on a cycle after it is made (issue
            // 1331). Interrupts are asynchronous to the program, so a
            // cycle later is nothing a program can tell; what could be
            // told is one taken that the state has just forbidden, so
            // the registered decision is dropped after anything that
            // changes what may be taken, below.
            let int_now = int_set != 0;
            let int_ok = self.int_q.get();
            let int_m = self.int_m_q.get();
            let stall_m = m_here & !int_ok & !m_done;
            // A `fence` orders what came before it against what comes
            // after: it waits in execute until every store the core
            // has posted has been answered, and a load already waits
            // for its answer, so nothing else is outstanding. Both
            // fences share the opcode and wait alike.
            let is_fence = opcode == 0x0f;
            // `fence.i` waits as `fence` does and then refetches the
            // next instruction with the fetch's buffer dropped, so that
            // what runs is what memory holds (issue 1096).
            let is_fencei = is_fence & (f3 == 1);
            // `sfence.vma` waits as a fence does, so that the tables a
            // program wrote are in memory before a walk reads them
            // (issue 1014).
            let is_sfence = (opcode == 0x73)
                & (f3 == 0)
                & (ir.slice::<25, 7>() == 0x09)
                & (rd == 0);
            // The A extension's word forms (issue 1010): `lr.w`, `sc.w`
            // and the nine AMOs, by the five-bit function in bits 31 to
            // 27; `aq` and `rl` are taken as given, since every one of
            // them waits here as a fence does, for the stores posted
            // before it.
            let funct5 = ir.slice::<27, 5>();
            let is_aop = (opcode == 0x2f) & (f3 == 2);
            let is_lr = is_aop & (funct5 == 2) & (rs2 == 0);
            let is_sc = is_aop & (funct5 == 3);
            let is_rmw = is_aop
                & select!(funct5.raw() => {
                    0 | 1 | 4 | 8 | 12 | 16 | 20 | 24 | 28 => Bit::One,
                    _ => Bit::Zero,
                });
            let is_a = is_lr | is_sc | is_rmw;
            let stall_fence = self.valid
                & (Bit::from(is_fence) | is_a | Bit::from(is_sfence))
                & Bit::from(self.stores_out.get() != 0);
            // A load or store to the bus, which is every address, the
            // boot memory included. A store goes out when the bus has room; a
            // load goes out and moves on to writeback, which holds it
            // until the answer has landed in its register there.
            let ra = self.regs.read(self.ra_at.get());
            let a = mux(rs1 == 0, U::<32>::from(0u32), mux(fwd_a, wb_alu, ra));
            let b = mux(
                rs2 == 0,
                U::<32>::from(0u32),
                mux(fwd_b, wb_alu, self.regs.read(rs2)),
            );
            let off = select!(opcode.raw() => {
                0x23 => imm_s,
                0x2f => U::<32>::from(0u32),
                _ => imm_i,
            });
            let addr = a + off;
            // Whether the access is in the data RAM's window (issue 1275),
            // without the full sum: the offset is twelve bits, signed, so
            // the sum's upper half is rs1's, less one when the offset is
            // negative, plus the carry out of the lower halves' sum. That
            // sum, seventeen bits, also gives the lanes their address,
            // so the window and the address share one short carry chain.
            // Under translation, the physical address decides.
            let lo17 = U::<1>::from(0u8).concat::<_, 17>(a.slice::<0, 16>())
                + U::<1>::from(0u8).concat::<_, 17>(off.slice::<0, 16>());
            let c16 = lo17.bit(16);
            let hi = a.slice::<16, 16>();
            let in_bare = mux(
                off.bit(31),
                (Bit::from(hi == 2) & !c16) | (Bit::from(hi == 1) & c16),
                (Bit::from(hi == 1) & !c16) | (Bit::from(hi == 0) & c16),
            );
            let x_pa = self.x_pa.get();
            let local = mux(
                self.x_done,
                Bit::from(x_pa.slice::<16, 16>() == 1),
                in_bare,
            );
            // The lanes' word, within their depth: the board's 16384
            // words, or the few the document's and the layout's netlist
            // keep, so that 64 KiB of flip-flops is not mapped onto cells
            // (issue 1275).
            let dl_mask = U::<14>::from((DW - 1) as u32);
            let dl_at =
                mux(self.x_done, x_pa.slice::<2, 14>(), lo17.slice::<2, 14>())
                    & dl_mask;
            // `lr.w` and an AMO read as a load does, and the AMO's store
            // comes later, from writeback; `sc.w` stores as a store
            // does, if its reservation holds. The reservation is
            // compared with the register, not the sum, since an A
            // instruction adds no offset, which keeps the compare off
            // the adder's path.
            let is_load = (opcode == 0x03) | is_lr | is_rmw;
            let is_store = (opcode == 0x23) | is_sc;
            let sc_hit =
                self.rsv_valid & (self.rsv_at.get() == a.slice::<2, 30>());
            // What the `sc.w` acts on: the compare a cycle old, made
            // while it waited (issue 1298).
            let sc_ok = self.sc_q.get();
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
            // Every address is on the bus, the boot memory included:
            // it sits there read-only at zero, so a load can reach a
            // constant beside the code (issue 268).
            let here = self.valid & !rst & !self.stopped & !in_debug;
            // A load or a store waits for room on the bus whatever its
            // address, so that the fetch's hold does not hang on the
            // address's decode, which was the path that limited the
            // clock; there is room whenever the devices keep up, and
            // they do. A device load's wait for its answer is a
            // register, so the hold for it is state too.
            //
            // Under translation (issue 1014) the access waits here for
            // its physical address, which the unit answers from the
            // address register a cycle or more later, and while a walk
            // reads the bus it waits for that, since only one read is
            // ever out.
            //
            // A misaligned access waits the same way and traps when it
            // runs, so the wait does not hang on the adder: one term,
            // of the decode and registers, is all the address costs the
            // fetch's hold. Its translation is asked for too, and is
            // not used, since the misaligned trap comes first.
            //
            // A load waits as well while a fetch is out, since only one
            // read is: the fetch ahead of time (issue 1187) is the one
            // fetch that can be out under an instruction ready to run.
            let stall_mem = here
                & (is_load | is_store)
                & (!(issue.ready() & wbeat.ready())
                    | self.p_wait
                    | (is_load & self.f_wait)
                    // Without the data cache every load goes straight to the
                    // bus, so it waits for the stores posted before it,
                    // which the cache's path does on its own (issue 1463);
                    // with the cache the term is constant zero.
                    | (is_load
                        & Bit::from(IC == 0)
                        & Bit::from(self.stores_out.get() != 0))
                    | (dvm & !self.x_done));
            // A word of the instruction is still on the bus: the core
            // waits for it, which is what makes a program above the
            // boot memory slow and correct.
            let need1 = far & odd & !short;
            // A word the bus refused is fetched as zero, which decodes as
            // an illegal instruction and so runs nothing; the mark goes
            // with it into execute, where the cause is named.
            // A page fault the same way (issue 1014): the first word's
            // comes first, then an access fault in it, then the second
            // word's of either.
            let f_pf = far & (self.f_pf0 | (!self.f_bad0 & need1 & self.f_pf1));
            let f_fault = far & !f_pf & (self.f_bad0 | (need1 & self.f_bad1));
            let fetched = mux(f_fault | f_pf, U::<32>::from(0u32), fetched);
            let f_ready = !far | (hit0 & (!need1 | hit1));
            let stall_fetch = !f_ready;
            // `wfi` holds the core here until an interrupt is pending
            // and enabled. The wait does not ask whether interrupts
            // are enabled globally: the specification lets a core
            // resume with `mstatus.MIE` clear, and a kernel idles
            // inside its own interrupt lock, so a wait that waited for
            // the global bit would never end. A halt request ends it
            // too, as the debug specification asks: a kernel idles in
            // `wfi`, and a debugger that could not stop it there could
            // not attach to it (issue 930). The `wfi` has retired, so
            // the core halts on the instruction after it.
            let wake = (pend != 0) | haltreq;
            // A CSR instruction waits its first cycle in execute: its CSR
            // is found from the instruction register, a register, and so
            // is ready a cycle after the word arrives (issue 1295).
            // An `sc.w` waits until last cycle's compare was its own,
            // with its operand valid (issue 1298).
            let stall_sc = self.valid & is_sc & !self.sc_ready & !in_debug;
            let stall_csr = self.valid
                & Bit::from(opcode == 0x73)
                & Bit::from(f3 != 0)
                & self.ir_new
                & !in_debug;
            self.stall.set(
                stall_ld
                    | stall_csr
                    | stall_sc
                    | stall_m
                    | stall_fence
                    | stall_mem
                    | stall_fetch
                    | self.dev_wait
                    | amo_busy
                    | scx_wait
                    | self.waiting,
            );
            let stall = self.stall.get();
            // The address after the instruction, which a jump links:
            // two bytes on for a compressed one, four for the rest.
            let link =
                pc + mux(self.ir_c, U::<32>::from(2u32), U::<32>::from(4u32));
            // Live: an instruction in execute that is not stalled. It
            // runs unless the interrupt takes its place.
            let live = !rst
                & !self.stopped
                & self.valid
                & !stall
                & !in_debug
                & !wb_fault;
            // Debug mode is entered before the instruction in execute,
            // as an interrupt is taken, and ahead of one: on a halt
            // request, on the instruction after a single step, or on an
            // `ebreak` that `dcsr.ebreakm` sends here rather than to the
            // trap (issue 154).
            let ebreakm = dcsr.bit(15);
            let is_ebreak =
                (opcode == 0x73) & (f3 == 0) & (ir.slice::<20, 12>() == 1);
            let dbg_req = haltreq | self.stepped | (is_ebreak & ebreakm);
            self.dbg_take.set(live & dbg_req);
            let dbg_take = self.dbg_take.get();
            // A core that has stopped itself, by writing `mhalt`, is not
            // live, and so took no halt request either, which left a
            // debugger nothing to attach to after any program that had
            // ended (issue 1047). It enters debug mode on the request,
            // at the instruction after the write, where the stop left
            // the fetch, and the stop ends: a resume runs on from there.
            let stop_take = !rst & self.stopped & !in_debug & haltreq;
            // A store the bus refused is taken before the instruction,
            // as an interrupt is, ahead of one, and whether or not
            // interrupts are enabled: it is a trap.
            let st_take = live & self.st_err & !dbg_req;
            self.int_take.set(live & int_ok & !dbg_req & !self.st_err);
            let int_take = self.int_take.get();
            let run = live & !int_take & !dbg_take & !st_take;
            // Resume: once per request, to `dpc`, arming a single step
            // when `dcsr.step` asks for one.
            let resume_take = in_debug & resumereq & !self.resume_seen;
            // The access's translation faulted: it traps and goes
            // nowhere. Registers alone, since the answer only comes for
            // an access that is aligned (issue 1014).
            let xf = self.x_done & (self.x_pf | self.x_af);
            // A load in the data RAM's window goes to its lanes and not
            // to the bus (issue 1275). An AMO there waits a cycle in
            // writeback for its word, as one on the bus waits for its
            // answer.
            // A plain load whose address is in the DDR3 tries the data
            // cache (issue 1275), and so does every other plain load that
            // is not the data RAM's: writeback, which has the address,
            // reads around the cache when it is not the DDR3's, and that
            // read waits for every store the core has posted. A load sent
            // straight to the bus would not, and nothing between the core
            // and the DDR3 keeps a read behind a write to the same word
            // (issue 1463), so the cache is no longer chosen from rs1's top
            // bits: a base outside the DDR3 with an offset into it was a
            // load that could overtake a store.
            // The arrays are read at the page offset's bits, the same
            // before and after translation.
            let dc_on = U::<1>::from(IC as u32).bit(0);
            let dc_pred = mux(
                self.x_done,
                Bit::from(x_pa.slice::<30, 2>() == 1),
                Bit::from(a.slice::<30, 2>() == 1),
            );
            let dc_at = lo17.slice::<2, 10>();
            let dc_line_x = lo17.slice::<4, 8>();
            let dc_try = run
                & is_load
                & !is_lr
                & !is_rmw
                & !unaligned
                & !xf
                & !local
                & dc_on;
            // An AMO, lr.w or sc.w there goes around the cache and takes
            // its line out of it.
            let dc_own_x = run
                & (is_lr | is_rmw | is_sc)
                & !unaligned
                & !xf
                & !local
                & dc_pred;
            // And on a board whose arbiter and monitor keep exclusive
            // pairs, it is one: the read and the write go with `lock`
            // (issue 1408). An A instruction adds no offset, so the
            // prediction is the address's own.
            let xd = dc_pred & !local & Bit::from(EXCL != 0);
            let send_load = run & is_load & !unaligned & !xf & !local & !dc_try;
            let ld_loc = run & is_load & !unaligned & !xf & local;
            let amo_loc = ld_loc & is_rmw;

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
                0x37 | 0x17 | 0x6f | 0x67 | 0x03 | 0x13 | 0x33
                | 0x2f => Bit::One,
                0x73 => (f3 != 0).into(),
                _ => Bit::Zero,
            });
            // The system instructions. A CSR instruction reads one of
            // the registers `Csrs` holds and writes it, set or cleared
            // or replaced, from a register or a five-bit immediate;
            // ecall, ebreak and an instruction the core does not know
            // trap, ebreak entering debug mode instead when
            // `dcsr.ebreakm` says so; mret returns.
            let f12 = ir.slice::<20, 12>();
            let is_sys = opcode == 0x73;
            let csr_op = is_sys & (f3 != 0);
            let mip_now = mip_all;
            let csr_old = csr_read_at(
                self.csr_at.get(),
                Csrs {
                    mcycle: self.mcycle.get(),
                    minstret: self.minstret.get(),
                    mstatus,
                    mtvec,
                    mscratch: self.mscratch.get(),
                    mepc,
                    mcause: self.mcause.get(),
                    mie: mie_r,
                    mip: mip_now,
                    mtval: self.mtval.get(),
                    dcsr,
                    dpc,
                    busquiet: self.busquiet.get(),
                    medeleg: self.medeleg.get(),
                    mideleg,
                    counteren: self.counteren.get(),
                    stvec: self.stvec.get(),
                    sscratch: self.sscratch.get(),
                    menv_fiom: self.menv_fiom.get(),
                    senv_fiom: self.senv_fiom.get(),
                    sepc: self.sepc.get(),
                    scause: self.scause.get(),
                    stval: self.stval.get(),
                    satp: self.satp.get(),
                    time: time.get(),
                    hartid: U::<32>::from(HID as u32),
                },
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
            // `dcsr` and `dpc` are the debugger's: an instruction that names
            // either outside debug mode is illegal, a read as much as a
            // write, as the debug specification has it (issue 972). The
            // debug module reaches them through its own port, below.
            let dbg_only = (f12 == isa::CSR_DCSR) | (f12 == isa::CSR_DPC);
            // A CSR names in its bits 9 and 8 the least privilege that
            // may reach it, and a counter below machine mode wants its
            // enable, the machine's and in user mode the supervisor's
            // too (issue 1012).
            let is_ctr = (f12.slice::<8, 4>() == 0xc)
                & (f12.slice::<2, 5>() == 0)
                & (f12.slice::<0, 2>() != 3);
            let cen = self.counteren.get();
            let ctr_i = f12.slice::<0, 2>();
            // `hpmcounter3` to `31` and their upper halves: their bits
            // in `mcounteren` are zero for good, so only machine mode
            // reaches them (issue 1460).
            let is_hpm = (f12.slice::<8, 4>() == 0xc)
                & (f12.slice::<5, 2>() == 0)
                & (f12.slice::<0, 5>() >= 3);
            let ctr_ok = (!is_ctr
                | (((prv == 3) | bit_of(cen.zext::<32>(), ctr_i.zext::<5>()))
                    & ((prv != 0)
                        | bit_of(cen.zext::<32>(), ctr_i.zext::<5>() + 3))))
                & (!is_hpm | (prv == 3));
            let priv_ok = (prv >= f12.slice::<8, 2>()) & ctr_ok;
            // In supervisor mode, `mstatus.TVM` makes `satp` and
            // `sfence.vma` illegal, `TW` `wfi`, and `TSR` `sret`, so that
            // machine mode can take them over (issue 1347).
            let s_mode = Bit::from(prv == 1);
            let tvm = s_mode & mstatus.bit(20);
            let tw = s_mode & mstatus.bit(21);
            let tsr = s_mode & mstatus.bit(22);
            let csr_bad = (csr_ro(f12) & csr_writes)
                | (dbg_only & !in_debug)
                | !priv_ok
                | (Bit::from(f12 == isa::CSR_SATP) & tvm);
            let csr_src = mux(f3.bit(2), rs1.zext::<32>(), a);
            let csr_new = csr_value(f3, csr_old, csr_src);
            // A write to `mip` reads and modifies the software's SEIP and
            // not the controller's line, which a read shows ORed with it:
            // the line would otherwise latch into the software's bit on a
            // set or a clear of another bit (issue 1094).
            let sext = U::<32>::from(isa::SEXT);
            let mip_new = csr_value(
                f3,
                (csr_old & !sext) | (self.mip_sw.get() & sext),
                csr_src,
            );
            let sys0 = is_sys & (f3 == 0);
            let is_ecall = sys0 & (f12 == 0);
            // `is_ebreak` is decoded above, where debug entry needs it.
            let is_mret = sys0 & (f12 == 0x302);
            let is_sret = sys0 & (f12 == 0x102);
            let is_wfi = sys0 & (f12 == 0x105);
            let known = select!(opcode.raw() => {
                0x37 | 0x17 | 0x6f | 0x67 | 0x63 | 0x03 | 0x23 | 0x13 | 0x33
                | 0x0f => Bit::One,
                0x2f => is_a,
                0x73 => (csr_op & csr_known & !csr_bad)
                    | is_ecall
                    | is_ebreak
                    // `mret` only in machine mode, `sret` and `wfi` not in
                    // user mode (issue 1012).
                    | (is_mret & (prv == 3))
                    | (is_sret & (prv != 0) & !tsr)
                    | (is_wfi & (prv != 0) & !tw)
                    | (Bit::from(is_sfence) & (prv != 0) & !tvm),
                _ => Bit::Zero,
            });
            // A trap: ecall, a word the core does not know, or the
            // interrupt. The cause, the address and the trap value go to
            // the CSRs, the interrupt enable is saved and cleared, and
            // the handler is the redirect.
            //
            // The exceptions are the instruction's own; the interrupt and
            // the refused store are taken before it, from registers, and
            // redirect at once, while an exception redirects a cycle
            // later (issue 1195).
            let exc = run & (is_ecall | is_ebreak | !known | unaligned | xf);
            let trap = exc | int_take | st_take;
            // The exception an unaligned access raises says which way
            // it was going, and its trap value is the address, which is
            // what a handler emulating the access needs.
            let misaligned = mux(
                is_store | is_rmw,
                U::<32>::from(isa::CAUSE_STORE_MISALIGNED),
                U::<32>::from(isa::CAUSE_LOAD_MISALIGNED),
            );
            // The order the specification gives: the external
            // interrupt is taken before the software one, and that
            // before the timer's.
            let cause = mux(
                int_take,
                int_cause(self.int_set_q.get()),
                mux(
                    is_ecall,
                    // 8 from user mode, 9 from supervisor, 11 from machine.
                    U::<32>::from(8u32) + prv.zext::<32>(),
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
            let cause =
                mux(st_take, U::<32>::from(isa::CAUSE_STORE_ACCESS), cause);
            let fetch_bad = run & self.ir_bad;
            let cause =
                mux(fetch_bad, U::<32>::from(isa::CAUSE_FETCH_ACCESS), cause);
            // The page faults and a walk's access faults (issue 1014).
            let fetch_pf = run & self.ir_pf;
            let cause =
                mux(fetch_pf, U::<32>::from(isa::CAUSE_FETCH_PAGE), cause);
            let x_cause = mux(
                self.x_st,
                mux(
                    self.x_pf,
                    U::<32>::from(isa::CAUSE_STORE_PAGE),
                    U::<32>::from(isa::CAUSE_STORE_ACCESS),
                ),
                mux(
                    self.x_pf,
                    U::<32>::from(isa::CAUSE_LOAD_PAGE),
                    U::<32>::from(isa::CAUSE_LOAD_ACCESS),
                ),
            );
            // A misaligned access traps as one, whatever its page says.
            let cause = mux(run & xf & !unaligned, x_cause, cause);
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
            let tval = mux(st_take, U::<32>::from(0u32), tval);
            let tval = mux(run & self.ir_bad, pc, tval);
            // A page fault's value is the virtual address that
            // faulted: the instruction's, or its second half's when that
            // is on the next page, or the access's (issue 1014).
            let tval = mux(
                fetch_pf,
                mux(self.ir_pf2, pc + U::<32>::from(2u32), pc),
                tval,
            );
            let tval = mux(run & xf, self.x_va.get(), tval);
            // Where a trap goes (issue 1012): below machine mode, to the
            // supervisor when machine mode delegated its cause, in
            // `mideleg` for an interrupt and `medeleg` for an exception,
            // and else to machine mode. Each saves its enable and the
            // mode it came from, and `mret` and `sret` restore them and
            // leave user mode behind in their place.
            let zero32 = U::<32>::from(0u32);
            //
            // The choice is on the redirect's path, so it is not made
            // from the cause, which settles late, behind the address's
            // adder: an interrupt goes to the supervisor exactly when no
            // interrupt machine mode keeps is pending and enabled, which
            // is registers alone, and each exception's bit of `medeleg`
            // is read beforehand, so that what settles late only chooses
            // between two of them.
            let medeleg = self.medeleg.get();
            let d_ecall = mux(prv == 1, medeleg.bit(9), medeleg.bit(8));
            let d_mis = mux(is_store | is_rmw, medeleg.bit(6), medeleg.bit(4));
            let d_xf = mux(
                self.x_st,
                mux(self.x_pf, medeleg.bit(15), medeleg.bit(7)),
                mux(self.x_pf, medeleg.bit(13), medeleg.bit(5)),
            );
            // The choices made from registers and the decode come first,
            // the fetch's faults and the translation's among them, and
            // the two late ones choose last: a store's refusal, behind
            // the stall, and a misaligned access, behind the adder. A
            // misaligned access is none of the others but a translation's
            // fault, which it comes before: a word the fetch faulted on
            // decodes as no access (issue 1014).
            let d_early = mux(
                self.ir_pf,
                medeleg.bit(12),
                mux(
                    self.ir_bad,
                    medeleg.bit(1),
                    mux(
                        xf,
                        d_xf,
                        mux(
                            is_ecall,
                            d_ecall,
                            mux(is_ebreak, medeleg.bit(3), medeleg.bit(2)),
                        ),
                    ),
                ),
            );
            let d_exc =
                mux(st_take, medeleg.bit(7), mux(unaligned, d_mis, d_early));
            let to_s = (prv != 3) & mux(int_take, !int_m, d_exc);
            let wb_code = mux(
                self.wb_amo,
                U::<5>::from(isa::CAUSE_STORE_ACCESS as u8),
                U::<5>::from(isa::CAUSE_LOAD_ACCESS as u8),
            );
            let to_s_wb = (prv != 3) & bit_of(medeleg, wb_code);
            let stvec = self.stvec.get();
            // The trap's vector, on the way to the next program counter,
            // chosen last by whether the access is misaligned, which
            // settles behind the adder: the vector of every other trap
            // and that of a misaligned one are found beforehand, and the
            // check picks between them, one multiplexer from the fetch's
            // address (issue 1130). It is the vector `to_s` says.
            let to_s_other = (prv != 3)
                & mux(int_take, !int_m, mux(st_take, medeleg.bit(7), d_early));
            let vec_other = mux(to_s_other, stvec, mtvec);
            let vec_mis = mux((prv != 3) & d_mis, stvec, mtvec);
            let trap_vec =
                mux(unaligned & !int_take & !st_take, vec_mis, vec_other);
            let wb_vec = mux(to_s_wb, stvec, mtvec);
            let m_trap_status = (mstatus & !U::<32>::from(0x1888u32))
                | mux(mstatus.bit(3), U::<32>::from(0x80u32), zero32)
                | (prv.zext::<32>() << 11);
            let s_trap_status = (mstatus & !U::<32>::from(0x122u32))
                | mux(mstatus.bit(1), U::<32>::from(0x20u32), zero32)
                | mux(prv == 1, U::<32>::from(0x100u32), zero32);
            // A return below machine mode clears `MPRV` (issue 1105).
            let mret_status = (mstatus & !U::<32>::from(0x1808u32))
                & !mux(
                    mstatus.slice::<11, 2>() != 3,
                    U::<32>::from(0x2_0000u32),
                    zero32,
                )
                | U::<32>::from(0x80u32)
                | mux(mstatus.bit(7), U::<32>::from(0x8u32), zero32);
            let sret_status = (mstatus & !U::<32>::from(0x2_0102u32))
                | U::<32>::from(0x20u32)
                | mux(mstatus.bit(5), U::<32>::from(0x2u32), zero32);
            let mret_ok = run & is_mret & (prv == 3);
            let sret_ok = run & is_sret & (prv != 0) & !tsr;
            // Only an instruction that writes writes: a set or a clear
            // from `x0` or a zero immediate is a read, and wrote the
            // value it read back after the count, so `mcycle` and
            // `minstret` lost one on every read of them (#807).
            // An illegal one writes nothing either: `dcsr` named from
            // machine mode traps and stays as it was (issue 972).
            let csr_write = run & csr_op & csr_known & csr_writes & !csr_bad;
            // Stopped: on a write of an odd value to `mhalt`, and then
            // for good; the halt itself follows a cycle later, when
            // the halting instruction retires. `ebreak` used to do
            // this and now raises the breakpoint exception, so a
            // program that means to stop says so, which is issue 139.
            // A reset of the core ends the stop: the core declares
            // `rst` itself, so no register of its is put back by the
            // netlist, and a stop that survived a reset kept the core
            // halted for good, with the loader in the boot memory never
            // restarting (issue 398). The CSRs are put back by the same
            // line, below, which that fix left as they were (issue 419).
            //
            // `mhalt` reads as zero, so the value written has bit 0 set
            // exactly when the source's is and the operation is not a
            // clear: the halt does not wait for the CSR read's
            // multiplexers (issue 1195).
            let halt_src =
                mux(f3.slice::<0, 2>() == 3, Bit::Zero, csr_src.bit(0));
            let halting = csr_write & (f12 == isa::CSR_MHALT) & halt_src;
            let stop = mux(
                rst | stop_take,
                Bit::Zero,
                mux(run, halting, self.stopped.get()),
            );
            let wrote = run & writes & (rd != 0) & !trap;
            // `sc.w` stores only while its reservation holds.
            let store = run & is_store & !unaligned & (!is_sc | sc_ok) & !xf;
            // A flush: an `sfence.vma` that runs, or a write of `satp`.
            // Either is followed by a refetch of the next instruction,
            // under the translations as they now are (issue 1014).
            let flush_go = (run & Bit::from(is_sfence) & (prv != 0) & !tvm)
                | (csr_write & (f12 == isa::CSR_SATP));
            let refetch =
                Bit::from(is_sfence) | (csr_op & (f12 == isa::CSR_SATP));
            let fencei_go = run & Bit::from(is_fencei);
            let wval = select!(opcode.raw() => {
                0x37 => imm_u,
                0x17 => pc + imm_u,
                0x6f | 0x67 => link,
                0x73 => csr_old,
                // `sc.w` writes 0 when it stored and 1 when it did not.
                0x2f => mux(sc_ok, U::<32>::from(0u32), U::<32>::from(1u32)),
                _ => mux(is_m, m_res, alu),
            });
            // Where the instruction goes next, other than on: a jump's
            // or a taken branch's target, the saved address on mret, or
            // the next instruction after a refetch. An exception goes to
            // its handler a cycle later, from `tp_vec` (issue 1195), so
            // none of this waits for it; a jump that also traps goes
            // here first and is then squashed. The jalr arm is first
            // because it is last to settle: a loaded value forwarded
            // into the add, and this select is on the critical path.
            let target = select!(opcode.raw() => {
                0x67 => (a + imm_i) & !U::<32>::from(1u32),
                0x6f => pc + imm_j,
                0x63 => pc + imm_b,
                0x73 => mux(
                    is_mret,
                    mepc,
                    mux(is_sret, self.sepc.get(), link),
                ),
                _ => link,
            });
            // A conditional branch goes where it is guessed to: back is
            // taken, forward is not, from the immediate's sign alone, so
            // the compare is not on the way to the program counter; a
            // wrong guess is corrected a cycle later (issue 1300).
            let is_branch = Bit::from(opcode == 0x63);
            let guess = ir.bit(31);
            let wrong = run & is_branch & (taken ^ guess) & !exc;
            let right_at = mux(taken, pc + imm_b, link);
            let jump = select!(opcode.raw() => {
                0x6f | 0x67 => Bit::One,
                0x63 => guess,
                0x73 => is_mret | is_sret | refetch,
                0x0f => Bit::from(is_fencei),
                _ => Bit::Zero,
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
            // The register file's write port: the writeback's, or the
            // debug module's while the core is halted and nothing
            // retires; `x0` is not written for it either.
            let dbg_gpr_we = in_debug & dbg_we & dbg_is_gpr & (dbg_gpr != 0);
            let rf_we = wb_write | dbg_gpr_we;
            let rf_at = mux(dbg_gpr_we, dbg_gpr, wb_rd);
            let rf_val = mux(dbg_gpr_we, dbg_wdata, wb_val);
            when!(rf_we => self { regs.at(rf_at): rf_val });
            self.halted
                .set(!rst & !stop_take & (self.halted | self.wb_stop));
            // The bus: a load or a store is a burst of one beat at
            // the address, and a store's beat carries the data with
            // the lanes it covers as its strobe. The load's wait is a
            // register. The identifier is the tracker's to allocate,
            // so the core writes none, and the grant it sends back is
            // of no use here and is dropped.
            let _ = grant.recv_if(grant.peek().is_some());
            let send_store = store & !local;
            let st_loc = store & local;
            // An AMO's store, from writeback, once its word is computed
            // and the bus has room; nothing of execute's goes out then,
            // since execute waits for it, and it goes before a fetch.
            let amo_go = (amo_ph == 2) & issue.ready() & wbeat.ready();
            // An exclusive AMO that did not store reads again, once
            // nothing else of the core's is out (issue 1408).
            let amo_rd_go =
                (amo_ph == 4) & !self.f_wait & !self.p_wait & issue.ready();
            // From when it is to go until its answer, no other read of
            // the core's goes out, so the answer is its own.
            let amo_rd_busy = (amo_ph == 4) | (amo_ph == 5);
            let amo_loc_go = amo_go & self.wb_loc;
            let st_go = send_store | (amo_go & !self.wb_loc);
            // A store into the data RAM, and an AMO's store there, write
            // its lanes in the cycle they go (issue 1275): the store's
            // lanes under its strobes, the AMO's whole word.
            let dl_we = st_loc | amo_loc_go;
            let dl_wa =
                mux(amo_loc_go, self.wb_pa.get().slice::<2, 14>(), dl_at)
                    & dl_mask;
            let dl_wd = mux(amo_loc_go, self.amo_val.get(), sdata);
            let dl_en = mux(amo_loc_go, U::<4>::from(15u8), en);
            let dw0 = dl_we & dl_en.bit(0);
            let dw1 = dl_we & dl_en.bit(1);
            let dw2 = dl_we & dl_en.bit(2);
            let dw3 = dl_we & dl_en.bit(3);
            when!(dw0 => self { dl0.at(dl_wa): dl_wd.slice::<0, 8>() });
            when!(dw1 => self { dl1.at(dl_wa): dl_wd.slice::<8, 8>() });
            when!(dw2 => self { dl2.at(dl_wa): dl_wd.slice::<16, 8>() });
            when!(dw3 => self { dl3.at(dl_wa): dl_wd.slice::<24, 8>() });
            // A fetch goes out when the words it wants are not in the
            // buffer, nothing else of the core's is out, and the
            // channel has room. The second word is asked for after the
            // first, since only the first says whether it is wanted.
            // The word after the buffer's one is asked for ahead of time
            // when it is in the same line (issue 1187), so a taken branch
            // never waits behind the fill of a line it did not want.
            let next_in_line = Bit::from(f_at4.slice::<2, 2>() != 0);
            let f_want = far & (!hit0 | (!hit1 & (need1 | next_in_line)));
            let f_addr = mux(hit0, want + 4, want);
            // Under translation (issue 1014) a fetch goes out once the
            // page of the word it wants is translated, at the physical
            // address; a page that faults is not read, and its words
            // are taken as faults at once.
            //
            // Whether the page is the one translated is found for both
            // words at once, the next word's against the page after when
            // the word is a page's last, and `hit0` chooses, so the
            // compare does not wait for the add (issue 1130).
            let want_vpn = want.slice::<12, 20>();
            let ft_vpn = self.ft_vpn.get();
            let same0 = Bit::from(ft_vpn == want_vpn);
            let last_word = Bit::from(want.slice::<2, 10>() == 0x3ff);
            let same1 =
                mux(last_word, Bit::from(ft_vpn == want_vpn + 1), same0);
            let f_th = self.ft_valid & mux(hit0, same1, same0);
            let f_tf = self.ft_pf | self.ft_af;
            let f_pa = mux(
                vm,
                self.ft_ppn.get().concat::<_, 32>(f_addr.slice::<0, 12>()),
                f_addr,
            );
            let f_send = f_want
                & (!vm | (f_th & !f_tf))
                & !self.f_wait
                & !self.p_wait
                & !self.dev_wait
                & !send_load
                & !send_store
                & !amo_go
                & !amo_rd_busy
                & issue.ready();
            let f_ffill = f_want & vm & f_th & f_tf & !self.f_wait;
            // The instruction cache (issue 1021). A fetch from the data
            // memory or DDR3 goes to the cache instead of the bus: it
            // looks its word up the cycle after, and a miss fills the
            // line from the bus as one burst of four words while the
            // fetch waits. Whether the page is one of those is found for
            // both words at once, as `f_th` is, so that the choice does
            // not wait for the add.
            let c_page0 = cacheable(want_vpn);
            let c_page1 = mux(last_word, cacheable(want_vpn + 1), c_page0);
            // IC is 1 on the board and 0 in the netlist the documents
            // simulate and the layout maps: there no fetch goes to the
            // cache and its arrays are never written, so they are
            // constants the synthesis folds away, since a cell library
            // has no RAM and 16 KiB of flip-flops would be most of the
            // layout (issue 1323). The writes are gated as well as the
            // fetch, since the synthesis does not follow the cache's
            // state through its registers to see that it never fills.
            let ic_on = U::<1>::from(IC as u32).bit(0);
            let f_cache = mux(
                vm,
                cacheable(self.ft_ppn.get()),
                mux(hit0, c_page1, c_page0),
            ) & ic_on;
            let c_go = f_send & f_cache;
            let b_go = f_send & !f_cache;
            let ic_st = self.ic_st.get();
            let ic_pa = self.ic_pa.get();
            let ic_clearing = self.ic_clearing.get();
            // The lookup reads the line's tag and the word at the
            // address registered when the fetch went to the cache.
            let ic_line = ic_pa.slice::<4, 10>();
            let ic_t = self.ic_tdat.get();
            // The word, and the one after it, which a hit puts in the
            // buffer's second place when it is in the same line (issue
            // 1187): one from each bank, each at its own address
            // register (issue 1303).
            let ic_ew = self.ic_ewd.get();
            let ic_ow = self.ic_owd.get();
            let ic_w = mux(ic_pa.bit(2), ic_ow, ic_ew);
            let ic_w1 = mux(ic_pa.bit(2), ic_ew, ic_ow);
            // Where the banks are read: the line, and each bank's word
            // in it. The odd bank's is bit 3 of the address; the even
            // bank's is the word's own when it is even, the one after it
            // when it is odd. At the fetch's address while the cache is
            // idle, so that the read is there when the fetch goes to it,
            // and at the registered one after a fill.
            let rd_pa = mux(Bit::from(ic_st == 0), f_pa, ic_pa);
            let rd_line = rd_pa.slice::<4, 10>();
            let rd_ea = rd_line
                .concat::<_, 11>(rd_pa.slice::<3, 1>() | rd_pa.slice::<2, 1>());
            let rd_oa = rd_line.concat::<_, 11>(rd_pa.slice::<3, 1>());
            let ic_clr2 = self.ic_clr2.get();
            let rd_go = Bit::from(ic_st == 0)
                | Bit::from(ic_st == 3)
                | (Bit::from(ic_st == 1) & (ic_clearing | ic_clr2));
            let looking = Bit::from(ic_st == 1);
            let filling = Bit::from(ic_st == 2);
            let ic_hit = ic_t.bit(20)
                & Bit::from(ic_t.slice::<0, 20>() == ic_pa.slice::<12, 20>());
            // A lookup waits while the tags are cleared, ends when its
            // fetch was dropped, fills the buffer on a hit, and on a miss
            // asks the bus for the line once nothing else of the core's
            // is out or going, so that the four beats are the only
            // answers until the last.
            let l_live = looking & !ic_clearing & !ic_clr2;
            let l_drop = l_live & self.f_drop;
            let l_hit = l_live & !self.f_drop & ic_hit;
            let r_go = l_live
                & !self.f_drop
                & !ic_hit
                & !self.p_wait
                & !self.dev_wait
                & !send_load
                & !st_go
                & !amo_rd_busy
                & issue.ready();
            let line_base = ic_pa & U::<32>::from(0xffff_fff0u32);
            // A walk's read goes out when nothing else of the core's is
            // out or going and every store is answered, so that it reads
            // what the program wrote.
            let p_send = ptw.peek().is_some()
                & !self.f_wait
                & !self.p_wait
                & !self.dev_wait
                & !send_load
                & !st_go
                & !f_send
                & !r_go
                & !amo_rd_busy
                & Bit::from(self.stores_out.get() == 0)
                & issue.ready();
            let p_addr = ptw.head();
            let _ = ptw.recv_if(p_send);
            // The data cache's miss asks for its line (issue 1275) once
            // every store of the core's is answered, so that the line has
            // them, and nothing else of the core's is out; a single word
            // for a load not in the DDR3 after all. That word, a device's,
            // does not wait for the posted stores (issue 1463): no store of
            // the core's to a device is kept in order with a load from one
            // anywhere else either, and the DDR3's data cannot be there.
            let wb_pa = self.wb_pa.get();
            let dc_st = self.dc_st.get();
            let dc_ask = self.dev_wait
                & self.wb_dc
                & Bit::from(dc_st == 1)
                & !self.f_wait
                & !self.p_wait
                & (Bit::from(self.stores_out.get() == 0) | self.dc_one)
                & issue.ready();
            let dc_line_go = dc_ask & !self.dc_one;
            let send_any =
                send_load | st_go | b_go | r_go | p_send | dc_ask | amo_rd_go;
            // An exclusive read or write (issue 1408): `lr.w` and an AMO's
            // read, `sc.w`, an AMO's store, and its read again.
            let x_lock =
                ((send_load & (is_lr | is_rmw)) | (send_store & is_sc)) & xd;
            let lock = x_lock | (amo_go & self.wb_x) | amo_rd_go;
            // The address: the access's, which settles last behind its
            // adder, goes through one choice, and the rest are chosen
            // among beforehand.
            let other = mux(
                p_send,
                p_addr,
                mux(
                    b_go,
                    f_pa,
                    mux(
                        r_go,
                        line_base,
                        mux(
                            amo_go | amo_rd_go,
                            self.wb_pa.get(),
                            mux(
                                dc_ask,
                                mux(
                                    self.dc_one,
                                    wb_pa,
                                    wb_pa & U::<32>::from(0xffff_fff0u32),
                                ),
                                self.x_pa.get(),
                            ),
                        ),
                    ),
                ),
            );
            let use_addr = !p_send
                & !b_go
                & !r_go
                & !amo_go
                & !amo_rd_go
                & !dc_ask
                & !self.x_done;
            if bool::from(send_any) {
                issue.send(Issue {
                    read: !st_go,
                    addr: mux(use_addr, addr, other),
                    // A fill is a burst of the line's four words.
                    len: mux(
                        r_go | dc_line_go,
                        U::<8>::from(3u8),
                        U::<8>::from(0u8),
                    ),
                    size: U::<3>::from(2u8),
                    burst: BurstKind::Incr,
                    lock,
                    cache: U::<4>::from(0u8),
                    prot: U::<3>::from(0u8),
                    qos: U::<4>::from(0u8),
                    region: U::<4>::from(0u8),
                });
            }
            if bool::from(st_go) {
                wbeat.send(W {
                    data: mux(amo_go, self.amo_val.get(), sdata),
                    strb: mux(amo_go, U::<4>::from(15u8), en),
                    last: Bit::One,
                });
            }
            if bool::from(take_done | (take_r & rh.last)) {
                release.send(Grant {
                    id: mux(take_done, dh.id, rh.id),
                });
            }
            // An answer belongs to whichever of the three is out, and
            // only one ever is: the fetch's, the walker's, or the
            // data's. The fetch's is a word from the bus, or a beat of
            // the cache's fill; a fetch the cache is looking up is not
            // on the bus at all.
            let f_resp = resp_valid & self.f_wait & Bit::from(ic_st == 0);
            let r_beat = resp_valid & filling;
            let f_bus = (self.f_wait & Bit::from(ic_st == 0)) | filling;
            let p_resp = resp_valid & self.p_wait;
            let d_resp = resp_valid & !f_bus & !self.p_wait;
            // The data cache in writeback (issue 1275). The load's first
            // cycle there is the lookup, on the tag and the word read at
            // its address in execute; a hit takes the word. A miss takes
            // the line out, asks for it, writes its beats in as they come,
            // takes the load's word as it passes, and keeps the line on
            // the last beat unless another host's write, a refused beat or
            // a store of the core's came to it meanwhile.
            let wb_line = wb_pa.slice::<4, 8>();
            let dc_in = Bit::from(wb_pa.slice::<30, 2>() == 1);
            // What execute read, with what the cache took in the same
            // cycle over it, each compare of two registers (issue 1431).
            let dc_kept =
                self.dc_fk_v & Bit::from(self.dc_fk_line.get() == wb_line);
            let dc_gone = (self.dc_fs_v
                & Bit::from(self.dc_fs_line.get() == wb_line))
                | (self.dc_fo_v & Bit::from(self.dc_fo_line.get() == wb_line))
                | (self.dc_fm_v & Bit::from(self.dc_fm_line.get() == wb_line));
            let dc_vr_now = !self.dc_fall & ((self.dc_vr & !dc_gone) | dc_kept);
            let dc_tag_now =
                mux(dc_kept, self.dc_fk_tag.get(), self.dc_tagr.get());
            let dc_word_now = mux(
                self.dc_fd_v
                    & Bit::from(self.dc_fd_a.get() == wb_pa.slice::<2, 10>()),
                self.dc_fd.get(),
                self.dc_word.get(),
            );
            let dc_tag_hit =
                dc_vr_now & Bit::from(dc_tag_now == wb_pa.slice::<12, 20>());
            let dc_look = self.dev_wait & self.wb_dc & Bit::from(dc_st == 0);
            let dc_hit = dc_look & dc_in & dc_tag_hit;
            let dc_miss = dc_look & !dc_hit;
            let dc_fill = Bit::from(dc_st == 2);
            let dc_beat_r = d_resp & dc_fill;
            let dc_want = self.dc_one
                | Bit::from(self.dc_beat.get() == wb_pa.slice::<2, 2>());
            let dc_last = dc_beat_r & rh.last;
            // Another host's write into the DDR3, answered (issue 1275).
            let snoop = dc_snoop.get();
            let snoop_v = snoop.bit(8) & dc_on;
            let snoop_line = snoop.slice::<0, 8>();
            let dc_kill_now = self.dc_kill
                | (snoop_v & Bit::from(snoop_line == wb_line))
                | (dc_beat_r & resp_bad);
            let dc_keep = dc_last & !self.dc_one & !dc_kill_now & dc_on;
            let dc_drop = dc_last & !self.dc_one & dc_kill_now;
            // A store of the core's in writeback writes its word into a
            // line the cache holds, under its lanes.
            let dc_st_up = self.wb_valid & self.wb_st & dc_in & dc_tag_hit;
            let dc_dwe = ((dc_beat_r & !self.dc_one) | dc_st_up) & dc_on;
            let dc_da = mux(
                dc_st_up,
                wb_pa.slice::<2, 10>(),
                wb_line.concat::<_, 10>(self.dc_beat.get()),
            );
            let st_mask = lane_mask(self.wb_en.get());
            let dc_dd = mux(
                dc_st_up,
                (dc_word_now & !st_mask) | (self.wb_sd.get() & st_mask),
                resp_data,
            );
            when!(dc_dwe => self { dc_data.at(dc_da): dc_dd });
            when!(dc_keep => self { dc_tag.at(wb_line): wb_pa.slice::<12, 20>() });
            // The valid bits: cleared by another host's write, by an AMO,
            // lr.w or sc.w, by a miss for its line and by a fill not kept;
            // set by a fill kept; and all cleared at reset and when the bus
            // refuses a store of the core's, whose line is not known.
            // A hart other than the first, whose cache the other
            // harts' writes are not named to while it waits in `wfi`,
            // forgets its cache as it wakes, whatever woke it (issue
            // 1408).
            let dc_all = rst
                | (take_done & done_bad & !self.busquiet)
                | (wake & self.waiting & Bit::from(HID != 0));
            let dc_miss_in = dc_miss & dc_in;
            let dc_clr0 = dc_bit(snoop_line, snoop_v, Bit::Zero)
                | dc_bit(dc_line_x, dc_own_x, Bit::Zero)
                | dc_bit(wb_line, dc_miss_in | dc_drop, Bit::Zero);
            let dc_clr1 = dc_bit(snoop_line, snoop_v, Bit::One)
                | dc_bit(dc_line_x, dc_own_x, Bit::One)
                | dc_bit(wb_line, dc_miss_in | dc_drop, Bit::One);
            let dc_live =
                mux(dc_all, U::<1>::from(0u8), U::<1>::from(1u8)).sext::<128>();
            let dc_v0n = ((self.dc_v0.get() & !dc_clr0)
                | dc_bit(wb_line, dc_keep, Bit::Zero))
                & dc_live;
            let dc_v1n = ((self.dc_v1.get() & !dc_clr1)
                | dc_bit(wb_line, dc_keep, Bit::One))
                & dc_live;
            if bool::from(p_resp) {
                pte.send(Pte {
                    data: resp_data,
                    err: resp_bad,
                });
            }
            // A change of the translations: a flush, or, with
            // translation on in `satp`, a change of mode, which a trap,
            // a return or a resume may be. The fetch's translation and
            // its buffer go, and a fetch out on the bus is dropped when
            // it answers (issue 1014). A `fence.i` drops them the same
            // way, so that the code it fetches next is what memory holds
            // (issue 1096).
            let vctx = flush_go
                | fencei_go
                | (satp_r.bit(31)
                    & (trap | mret_ok | sret_ok | resume_take | wb_fault));
            // The fill's last beat: the line becomes valid and is looked
            // up again, unless the fetch was dropped meanwhile or a beat
            // was refused, when the fetch ends, the refusal taken as the
            // word's access fault.
            let r_last = r_beat & Bit::from(self.ic_beat.get() == 3);
            let r_bad = self.ic_bad | resp_bad;
            let r_stop = r_last & (self.f_drop | r_bad);
            let r_badend = r_last & !self.f_drop & r_bad;
            let r_ok = r_last & !self.f_drop & !r_bad;
            let c_end = l_hit | l_drop | r_stop;
            // A word into the fetch buffer: from the bus, from the
            // cache, or a fill's refusal.
            let b_fill = f_resp & !self.f_drop;
            let f_fill = b_fill | l_hit | r_badend;
            let f_word =
                mux(b_fill, resp_data, mux(l_hit, ic_w, U::<32>::from(0u32)));
            let f_err = mux(b_fill, resp_bad, r_badend);
            let second = self.f_second.get();
            let asked = self.f_asked.get();
            // A fill's beat into the line, and the tags' one write: a
            // clear, or a filled line made valid. A fill ending while
            // the tags are cleared stays invalid.
            // Each beat goes into its word's bank.
            let beat = self.ic_beat.get();
            let fill_at = ic_line.concat::<_, 11>(beat.slice::<1, 1>());
            let fill_even = r_beat & !beat.bit(0) & ic_on;
            let fill_odd = r_beat & beat.bit(0) & ic_on;
            when!(fill_even => self { ic_even.at(fill_at): resp_data });
            when!(fill_odd => self { ic_odd.at(fill_at): resp_data });
            let tag_go = (ic_clearing | r_ok) & ic_on;
            let tag_at = mux(ic_clearing, self.ic_clr.get(), ic_line);
            let tag_val = mux(
                ic_clearing,
                U::<21>::from(0u32),
                U::<1>::from(1u8).concat::<_, 21>(ic_pa.slice::<12, 20>()),
            );
            when!(tag_go => self { ic_tag.at(tag_at): tag_val });
            let d_word = d_resp & (!dc_fill | dc_want);
            when!(d_word => self { wb_dev: resp_data });
            when!(dc_hit => self { wb_dev: dc_word_now });
            // A load from the data RAM is never refused, so it clears
            // what a refused load before it left (issue 1275); a local
            // AMO's word is captured in its first cycle in writeback.
            let l_ans = self.dev_wait & self.wb_loc;
            when!(d_word | ld_loc | dc_hit => self { wb_err: resp_bad & d_word });
            when!(l_ans => self { wb_dev: self.dl_word.get() });
            // The fetch's request to the unit (issue 1014): made for the
            // page of the word the fetch wants when it is not the page
            // translated, held, and answered no sooner than two cycles
            // on, since the unit sees the request a cycle late in
            // simulation and on time in hardware, and an answer in
            // either is for the request the unit saw a cycle before.
            let (imr, dmr) = (ires.get(), dres.get());
            let i_age = self.i_age.get();
            let i_need = f_want & vm & !f_th & !self.f_wait;
            let i_take = self.i_req
                & Bit::from(i_age == 2)
                & (imr.ok | imr.fault | imr.err);
            // The data's the same way, for the access in execute, once
            // its operands are in.
            let x_age = self.x_age.get();
            let x_start = here
                & (is_load | is_store)
                & dvm
                & !self.x_done
                & !self.x_req
                & !stall_ld
                & !self.dev_wait
                & !amo_busy;
            let x_take = self.x_req
                & Bit::from(x_age == 2)
                & (dmr.ok | dmr.fault | dmr.err);
            // The counter moves on, at this edge, from an instruction in
            // the buffer's first word to one that begins in its second:
            // one that ends there, or a compressed one in its upper half
            // (issue 1279). The buffer shifts if it holds the second
            // word, and if that word is out, or going out now, it goes
            // into the first place when it comes back.
            let adv1 = far
                & hit0
                & (odd | !short)
                & !(stall | (stop & !run) | in_debug)
                & !f_ffill;
            let adv_out = adv1
                & Bit::from(f_have == 1)
                & ((self.f_wait & self.f_second) | f_send);
            let to_w0 = second & (self.f_sh | (adv_out & self.f_wait));
            with!(self <= {
                f_send ? {
                    f_wait: Bit::One,
                    f_asked: f_addr,
                    f_second: hit0
                },
                f_resp ? f_wait: Bit::Zero,
                f_fill & !second ? {
                    f_w0: f_word,
                    f_bad0: f_err,
                    f_pf0: Bit::Zero,
                    f_at: asked,
                    f_at4: asked + 4,
                    f_have: U::<2>::from(1u8)
                },
                f_fill & second ? {
                    f_w1: f_word,
                    f_bad1: f_err,
                    f_pf1: Bit::Zero,
                    f_have: U::<2>::from(2u8)
                },
                // A hit for the first word fills the second as well
                // when the line holds it, so that a straight run takes
                // a lookup for every two words (issue 1187).
                l_hit & !second & Bit::from(ic_pa.slice::<2, 2>() != 3) ? {
                    f_w1: ic_w1,
                    f_bad1: Bit::Zero,
                    f_pf1: Bit::Zero,
                    f_have: U::<2>::from(2u8)
                },
                // The word out for the second place, when the buffer was
                // to shift: it goes into the first, and a hit fills the
                // word after it as well when the line holds it.
                f_fill & to_w0 ? {
                    f_w0: f_word,
                    f_bad0: f_err,
                    f_pf0: Bit::Zero,
                    f_at: asked,
                    f_at4: asked + 4,
                    f_have: U::<2>::from(1u8),
                    f_sh: Bit::Zero
                },
                l_hit & to_w0 & Bit::from(ic_pa.slice::<2, 2>() != 3) ? {
                    f_w1: ic_w1,
                    f_bad1: Bit::Zero,
                    f_pf1: Bit::Zero,
                    f_have: U::<2>::from(2u8)
                },
                f_fill ? f_sh: Bit::Zero,
                adv_out & !f_fill ? f_sh: Bit::One,
                f_ffill & !hit0 ? {
                    f_w0: U::<32>::from(0u32),
                    f_bad0: self.ft_af,
                    f_pf0: self.ft_pf,
                    f_at: f_addr,
                    f_at4: f_addr + 4,
                    f_have: U::<2>::from(1u8)
                },
                f_ffill & hit0 ? {
                    f_w1: U::<32>::from(0u32),
                    f_bad1: self.ft_af,
                    f_pf1: self.ft_pf,
                    f_have: U::<2>::from(2u8)
                },
                f_resp ? f_drop: Bit::Zero,
                // The cache's steps (issue 1021).
                rd_go ? {
                    ic_ewd: self.ic_even.read(rd_ea),
                    ic_owd: self.ic_odd.read(rd_oa),
                    ic_tdat: self.ic_tag.read(rd_line)
                },
                c_go ? ic_st: U::<2>::from(1u8),
                // The address is taken on every cycle the cache is idle,
                // not only when a fetch goes to it: nothing reads it then,
                // and a fetch goes to the cache only while it is idle, so
                // the value is the same, and its enable is the state alone,
                // with no stall in it (issue 1309).
                Bit::from(ic_st == 0) ? ic_pa: f_pa,
                r_go ? {
                    ic_st: U::<2>::from(2u8),
                    ic_beat: U::<2>::from(0u8),
                    ic_bad: Bit::Zero
                },
                r_beat ? {
                    ic_beat: self.ic_beat.get() + 1,
                    ic_bad: r_bad
                },
                // The line's words are read the cycle after its last
                // beat is written, and looked up the cycle after that.
                r_ok ? ic_st: U::<2>::from(3u8),
                Bit::from(ic_st == 3) ? ic_st: U::<2>::from(1u8),
                c_end ? {
                    ic_st: U::<2>::from(0u8),
                    f_wait: Bit::Zero,
                    f_drop: Bit::Zero
                },
                ic_clearing ? ic_clr: self.ic_clr.get() + 1,
                ic_clearing & Bit::from(self.ic_clr.get() == 1023) ?
                    ic_clearing: Bit::Zero,
                fencei_go ? {
                    ic_clearing: Bit::One,
                    ic_clr: U::<10>::from(0u8)
                },
                i_need & !self.i_req ? {
                    i_req: Bit::One,
                    i_va: f_addr,
                    i_age: U::<2>::from(0u8)
                },
                self.i_req & (i_age != 2) ? i_age: i_age + 1,
                i_take ? {
                    i_req: Bit::Zero,
                    ft_valid: Bit::One,
                    ft_vpn: self.i_va.get().slice::<12, 20>(),
                    ft_ppn: imr.pa.slice::<12, 20>(),
                    ft_pf: imr.fault,
                    ft_af: imr.err
                },
                x_start ? {
                    x_req: Bit::One,
                    x_va: addr,
                    x_age: U::<2>::from(0u8),
                    x_st: is_store | is_rmw
                },
                self.x_req & (x_age != 2) ? x_age: x_age + 1,
                x_take ? {
                    x_req: Bit::Zero,
                    x_done: Bit::One,
                    x_pa: dmr.pa,
                    x_pf: dmr.fault,
                    x_af: dmr.err
                },
                // The answer is the instruction's in execute, and goes
                // when it does.
                !stall | wb_fault ? {
                    x_req: Bit::Zero,
                    x_done: Bit::Zero
                },
                // The buffer shifts along, the second word and its
                // marks to the first, at the edge where the counter
                // moves on from an instruction in the first word to one
                // that begins in the second: one that ends there, or a
                // compressed one in its upper half. Nothing may be
                // filling it. A jump into the second word finds the
                // first word's address and fetches it again, which is
                // rare (issues 1187 and 1279).
                adv1 & Bit::from(f_have == 2) & !self.f_wait ? {
                    f_w0: self.f_w1.get(),
                    f_bad0: self.f_bad1.get(),
                    f_pf0: self.f_pf1.get(),
                    f_at: f_at4,
                    f_at4: f_at4 + 4,
                    f_have: U::<2>::from(1u8)
                },
                vctx ? {
                    f_have: U::<2>::from(0u8),
                    f_sh: Bit::Zero,
                    ft_valid: Bit::Zero,
                    i_req: Bit::Zero,
                    x_req: Bit::Zero
                },
                vctx & self.f_wait & !f_resp & !c_end ? f_drop: Bit::One,
                p_send ? p_wait: Bit::One,
                p_resp ? p_wait: Bit::Zero,
                flush: flush_go,
                // The data RAM's lanes, read at the address of the access
                // in execute, every cycle (issue 1275).
                dl_word: self
                    .dl3
                    .read(dl_at)
                    .concat::<_, 16>(self.dl2.read(dl_at))
                    .concat::<_, 24>(self.dl1.read(dl_at))
                    .concat::<_, 32>(self.dl0.read(dl_at)),
                // The data cache's word, tag and valid bit at the address
                // of the access in execute (issue 1275), and its valid
                // bits.
                dc_word: self.dc_data.read(dc_at),
                dc_tagr: self.dc_tag.read(dc_line_x),
                dc_vr: dc_vget(self.dc_v0.get(), self.dc_v1.get(), dc_line_x),
                dc_v0: dc_v0n,
                dc_v1: dc_v1n,
                dc_fd_v: dc_dwe,
                dc_fd_a: dc_da,
                dc_fd: dc_dd,
                dc_fk_v: dc_keep,
                dc_fk_line: wb_line,
                dc_fk_tag: wb_pa.slice::<12, 20>(),
                dc_fs_v: snoop_v,
                dc_fs_line: snoop_line,
                dc_fo_v: dc_own_x,
                dc_fo_line: dc_line_x,
                dc_fm_v: dc_miss_in | dc_drop,
                dc_fm_line: wb_line,
                dc_fall: dc_all,
                dc_miss ? {
                    dc_st: U::<2>::from(1u8),
                    dc_one: !dc_in,
                    dc_kill: Bit::Zero
                },
                dc_ask ? {
                    dc_st: U::<2>::from(2u8),
                    dc_beat: U::<2>::from(0u8)
                },
                dc_beat_r ? dc_beat: self.dc_beat.get() + 1,
                // A line named while its fill is out, before the first
                // beat as after it, is not kept (issue 1429); nor is one
                // whose fill is out as the whole cache is forgotten. A
                // wake cannot meet a fill today, since a fill holds
                // execute and wfi does not retire past one, but the cache
                // does not lean on that (issue 1408).
                Bit::from(dc_st != 0) ? dc_kill: dc_kill_now | dc_all,
                dc_last ? dc_st: U::<2>::from(0u8),
                ic_clr2: ic_clearing,
                rst ? {
                    ic_st: U::<2>::from(0u8),
                    ic_clearing: Bit::One,
                    ic_clr: U::<10>::from(0u8),
                    f_wait: Bit::Zero,
                    f_have: U::<2>::from(0u8),
                    f_sh: Bit::Zero,
                    f_bad0: Bit::Zero,
                    f_bad1: Bit::Zero,
                    f_pf0: Bit::Zero,
                    f_pf1: Bit::Zero,
                    f_drop: Bit::Zero,
                    ft_valid: Bit::Zero,
                    i_req: Bit::Zero,
                    x_req: Bit::Zero,
                    x_done: Bit::Zero,
                    p_wait: Bit::Zero,
                    flush: Bit::Zero,
                },
            });
            // What the unit is told: the translation's state, and the
            // two requests, all registers.
            mmu_satp.set(satp_r);
            // The data's mode, which is the fetch's but under `MPRV`, and
            // the fetch asks only below machine mode, where `MPRV` is
            // clear (issue 1105).
            mmu_prv.set(dprv);
            mmu_sum.set(mstatus.bit(18));
            mmu_mxr.set(mstatus.bit(19));
            mmu_flush.set(self.flush.get());
            ireq.set(IReq {
                req: self.i_req.get(),
                va: self.i_va.get(),
            });
            dreq.set(DReq {
                req: self.x_req.get(),
                va: self.x_va.get(),
                store: self.x_st.get(),
            });
            case!(rst => {
                Bit::One => { self.dev_wait <= Bit::Zero },
                _ if (send_load | amo_loc | dc_try).to_bool() => {
                    self.dev_wait <= Bit::One
                },
                _ if (self.dev_wait
                    & ((resp_valid & !self.wb_dc)
                        | self.wb_loc
                        | dc_hit
                        | dc_last))
                    .to_bool() =>
                {
                    self.dev_wait <= Bit::Zero
                },
                _ => {},
            });

            // The AMO's phases (issue 1010): the answer to its load, if
            // the bus gave the word, starts them; the new word is
            // computed from two registers, the old word and the
            // register operand, then stored, and the instruction then
            // retires with the old word. The reservation: `lr.w` makes
            // it and every `sc.w` uses it up, whether it stored or not.
            let amo_start = ((d_resp & !resp_bad) | l_ans) & self.wb_amo;
            // An exclusive pair's store waits for its answer, and reads
            // again when the answer says it did not store; an exclusive
            // `sc.w` waits for its answer, which is its result (issue
            // 1408). Every store before an A instruction is answered
            // before it goes, so the answer is this one's.
            let x_said = take_done & Bit::from(dh.resp == Resp::ExOkay);
            let x_ans = take_done & (amo_ph == 3);
            let scx_go = live & send_store & is_sc & xd;
            with!(self <= {
                amo_start ? amo_ph: U::<3>::from(1u8),
                amo_ph == 1 ? {
                    amo_val: amo_alu(
                        self.amo_op.get(),
                        self.wb_dev.get(),
                        self.amo_b.get(),
                    ),
                    amo_ph: U::<3>::from(2u8)
                },
                amo_go ? amo_ph: mux(
                    self.wb_x,
                    U::<3>::from(3u8),
                    U::<3>::from(0u8)
                ),
                x_ans ? amo_ph: mux(x_said, U::<3>::from(0u8), U::<3>::from(4u8)),
                amo_rd_go ? amo_ph: U::<3>::from(5u8),
                // A read again that the bus refused ends the AMO, which
                // retires as a refused load does: its error is taken.
                (amo_ph == 5) & d_resp & resp_bad ? amo_ph: U::<3>::from(0u8),
                scx_go ? scx_wait: Bit::One,
                (take_done & scx_wait) ? {
                    scx_wait: Bit::Zero,
                    scx_fail: !x_said
                },
                run & is_lr & !unaligned ? {
                    rsv_valid: Bit::One,
                    rsv_at: a.slice::<2, 30>()
                },
                run & is_sc ? rsv_valid: Bit::Zero,
                rst ? {
                    amo_ph: U::<3>::from(0u8),
                    rsv_valid: Bit::Zero,
                    scx_wait: Bit::Zero
                },
            });

            // The CSRs: written by a CSR instruction, by a trap, by mret.
            // The three never coincide in one instruction.
            // The external interrupt's pending bit is the line, taken
            // at each edge, and a write to `mip` does nothing: MEIP is
            // read only and the controller sets and clears it, as the
            // privileged specification has it (#788). It once latched
            // until software cleared it, which stock software never
            // does. A trap's writes come after the CSR writes, and win.
            with!(self <= {
                csr_write & (f12 == isa::CSR_MSTATUS) ?
                    mstatus: mstatus_w(csr_new),
                // User and supervisor mode (issue 1012).
                csr_write & (f12 == isa::CSR_SSTATUS) ?
                    mstatus: (mstatus & !U::<32>::from(SSTATUS_W))
                        | (csr_new & U::<32>::from(SSTATUS_W)),
                csr_write & (f12 == isa::CSR_SIE) ?
                    mie: (mie_r & !mideleg) | (csr_new & mideleg),
                csr_write & (f12 == isa::CSR_MIP) ?
                    mip_sw: mip_new & U::<32>::from(MIDELEG_W),
                csr_write & (f12 == isa::CSR_SIP) ?
                    mip_sw: (self.mip_sw.get() & !(mideleg & 2))
                        | (csr_new & mideleg & 2),
                csr_write & (f12 == isa::CSR_MEDELEG) ?
                    medeleg: csr_new & U::<32>::from(MEDELEG_W),
                csr_write & (f12 == isa::CSR_MIDELEG) ?
                    mideleg: csr_new & U::<32>::from(MIDELEG_W),
                csr_write & (f12 == isa::CSR_MCOUNTEREN) ?
                    counteren: self
                        .counteren
                        .get()
                        .slice::<3, 3>()
                        .concat::<_, 6>(csr_new.slice::<0, 3>()),
                csr_write & (f12 == isa::CSR_SCOUNTEREN) ?
                    counteren: csr_new
                        .slice::<0, 3>()
                        .concat::<_, 6>(self.counteren.get().slice::<0, 3>()),
                csr_write & (f12 == isa::CSR_STVEC) ?
                    stvec: csr_new & !U::<32>::from(3u32),
                csr_write & (f12 == isa::CSR_SSCRATCH) ? sscratch: csr_new,
                csr_write & (f12 == isa::CSR_MENVCFG) ?
                    menv_fiom: csr_new.bit(0),
                csr_write & (f12 == isa::CSR_SENVCFG) ?
                    senv_fiom: csr_new.bit(0),
                csr_write & (f12 == isa::CSR_SEPC) ?
                    sepc: csr_new & !U::<32>::from(1u32),
                csr_write & (f12 == isa::CSR_SCAUSE) ? scause: csr_new,
                csr_write & (f12 == isa::CSR_STVAL) ? stval: csr_new,
                csr_write & (f12 == isa::CSR_SATP) ? satp: csr_new,
                csr_write & (f12 == isa::CSR_MTVEC) ?
                    mtvec: csr_new & !U::<32>::from(3u32),
                csr_write & (f12 == isa::CSR_MSCRATCH) ? mscratch: csr_new,
                csr_write & (f12 == isa::CSR_MEPC) ?
                    mepc: csr_new & !U::<32>::from(1u32),
                csr_write & (f12 == isa::CSR_MCAUSE) ? mcause: csr_new,
                csr_write & (f12 == isa::CSR_MIE) ? mie: csr_new
                    & U::<32>::from(
                        isa::MEXT
                            | isa::MSOFT
                            | isa::MTIMER
                            | MIDELEG_W,
                    ),
                csr_write & (f12 == isa::CSR_MTVAL) ? mtval: csr_new,
                // The supervisor's external line beside it, SEIP, from the
                // controller's second target; a read shows it ORed with
                // the software's bit in `mip_sw` (issue 1094).
                mip: mux(irq, U::<32>::from(isa::MEXT), U::<32>::from(0u32))
                    | mux(
                        seirq.get(),
                        U::<32>::from(isa::SEXT),
                        U::<32>::from(0u32),
                    ),
                trap & !to_s ? {
                    mepc: pc,
                    mcause: cause,
                    mtval: tval,
                    mstatus: m_trap_status,
                    prv: U::<2>::from(3u8),
                },
                trap & to_s ? {
                    sepc: pc,
                    scause: cause,
                    stval: tval,
                    mstatus: s_trap_status,
                    prv: U::<2>::from(1u8),
                },
                // An AMO's refused word is a store's access fault.
                wb_fault & !to_s_wb ? {
                    mepc: self.wb_pc.get(),
                    mcause: wb_code.zext::<32>(),
                    mtval: wb_alu,
                    mstatus: m_trap_status,
                    prv: U::<2>::from(3u8),
                },
                wb_fault & to_s_wb ? {
                    sepc: self.wb_pc.get(),
                    scause: wb_code.zext::<32>(),
                    stval: wb_alu,
                    mstatus: s_trap_status,
                    prv: U::<2>::from(1u8),
                },
                st_take ? st_err: Bit::Zero,
                take_done & done_bad & !self.busquiet ? st_err: Bit::One,
                csr_write & (f12 == isa::CSR_MBUSQUIET) ?
                    busquiet: csr_new.bit(0),
                mret_ok ? {
                    mstatus: mret_status,
                    prv: mstatus.slice::<11, 2>(),
                },
                sret_ok ? {
                    mstatus: sret_status,
                    prv: mstatus.slice::<8, 1>().zext::<2>(),
                },
                // `wfi` retires and the core then waits. An interrupt
                // already pending means there is nothing to wait for,
                // and the wake below wins over the wait for that
                // reason: the two arms are written in that order.
                // The counters. `mcycle` runs while the core does and
                // stops with it: the halt stops the machine for good,
                // so a count that ran on afterwards would be a number
                // nobody could ever read. `minstret` counts what
                // retires, which is the same signal the writeback
                // drives and the lockstep steps on.
                !self.stopped ? mcycle: self.mcycle.get() + 1,
                // The stores out: one more as one goes, one fewer as one
                // is answered, and the count when both happen at once.
                st_go & !take_done ?
                    stores_out: self.stores_out.get() + 1,
                take_done & !st_go ?
                    stores_out: self.stores_out.get() - 1,
                wb_here ? minstret: self.minstret.get() + 1,
                // A write lands after the count, so a program that
                // sets a counter gets what it wrote rather than what
                // it wrote plus one.
                csr_write & (f12 == isa::CSR_MCYCLE) ?
                    mcycle: (self.mcycle.get()
                        & U::<64>::from(0xffff_ffff_0000_0000u64))
                        | csr_new.zext::<64>(),
                csr_write & (f12 == isa::CSR_MCYCLEH) ?
                    mcycle: (self.mcycle.get()
                        & U::<64>::from(0xffff_ffffu64))
                        | (csr_new.zext::<64>() << 32),
                csr_write & (f12 == isa::CSR_MINSTRET) ?
                    minstret: (self.minstret.get()
                        & U::<64>::from(0xffff_ffff_0000_0000u64))
                        | csr_new.zext::<64>(),
                csr_write & (f12 == isa::CSR_MINSTRETH) ?
                    minstret: (self.minstret.get()
                        & U::<64>::from(0xffff_ffffu64))
                        | (csr_new.zext::<64>() << 32),
                // Only a `wfi` that is legal waits: in user mode it traps
                // instead (issue 1012).
                run & is_wfi & (prv != 0) & !tw ? waiting: Bit::One,
                wake ? waiting: Bit::Zero,
                rst ? waiting: Bit::Zero,
                // Debug mode. On entry the cause says why, in bits 8
                // to 6: 1 an `ebreak`, 3 a halt request, 4 a step; the
                // version and the privilege are fixed, and `ebreakm`
                // and `step` are kept. A resume leaves, arming a step
                // when asked; one instruction later the step has run
                // and the next entry is requested.
                stop_take ? {
                    debug: Bit::One,
                    dpc: fetch_pc,
                    dcsr: U::<32>::from(0x4000_00c3u32)
                        | (dcsr & U::<32>::from(0x8004u32)),
                },
                dbg_take ? {
                    debug: Bit::One,
                    dpc: pc,
                    dcsr: U::<32>::from(0x4000_0000u32)
                        | prv.zext::<32>()
                        | (dcsr & U::<32>::from(0x8004u32))
                        | mux(
                            is_ebreak & ebreakm,
                            U::<32>::from(0x40u32),
                            mux(
                                self.stepped,
                                U::<32>::from(0x100u32),
                                U::<32>::from(0xc0u32),
                            ),
                        ),
                    stepped: Bit::Zero,
                },
                resume_take ? {
                    debug: Bit::Zero,
                    // The mode the hart was in, as `dcsr.prv` says.
                    prv: mux(
                        dcsr.slice::<0, 2>() == 2,
                        U::<2>::from(0u8),
                        dcsr.slice::<0, 2>(),
                    ),
                    step_armed: dcsr.bit(2),
                    resume_seen: Bit::One,
                },
                !resumereq ? resume_seen: Bit::Zero,
                run & self.step_armed ? {
                    step_armed: Bit::Zero,
                    stepped: Bit::One,
                },
                csr_write & (f12 == isa::CSR_DCSR) ?
                    dcsr: U::<32>::from(0x4000_0000u32)
                        | (csr_new & U::<32>::from(0x8004u32))
                        | (dcsr & U::<32>::from(0x1c3u32)),
                csr_write & (f12 == isa::CSR_DPC) ?
                    dpc: csr_new & U::<32>::from(0xffff_fffeu32),
                // The debug module writes the two, from outside, while
                // the core is halted.
                in_debug & dbg_we & !dbg_is_gpr & (dbg_csr == isa::CSR_DCSR) ?
                    dcsr: U::<32>::from(0x4000_0000u32)
                        | (dbg_wdata & U::<32>::from(0x8004u32))
                        | (dcsr & U::<32>::from(0x1c3u32)),
                in_debug & dbg_we & !dbg_is_gpr & (dbg_csr == isa::CSR_DPC) ?
                    dpc: dbg_wdata & U::<32>::from(0xffff_fffeu32),
                // A reset puts the CSRs back as configuration left
                // them, so a program started by the reset line sees
                // what a program started by configuration sees: the
                // previous program's trap vector and interrupt enables
                // would otherwise outlive it (issue 419). The
                // specification asks only for `mstatus.MIE` clear; the
                // rest is this design's decision, and the counters go
                // with them. The register file is not reset, which is
                // what a register file is.
                rst ? {
                    mstatus: U::<32>::from(0u32),
                    mtvec: U::<32>::from(0u32),
                    mscratch: U::<32>::from(0u32),
                    mepc: U::<32>::from(0u32),
                    mcause: U::<32>::from(0u32),
                    mie: U::<32>::from(0u32),
                    mip: U::<32>::from(0u32),
                    mtval: U::<32>::from(0u32),
                    mcycle: U::<64>::from(0u64),
                    minstret: U::<64>::from(0u64),
                    debug: Bit::Zero,
                    stepped: Bit::Zero,
                    step_armed: Bit::Zero,
                    resume_seen: Bit::Zero,
                    dcsr: U::<32>::from(0x4000_0003u32),
                    wb_err: Bit::Zero,
                    st_err: Bit::Zero,
                    dc_st: U::<2>::from(0u8),
                    dc_kill: Bit::Zero,
                    stores_out: U::<3>::from(0u8),
                    busquiet: Bit::Zero,
                    prv: U::<2>::from(3u8),
                    medeleg: U::<32>::from(0u32),
                    mideleg: U::<32>::from(0u32),
                    mip_sw: U::<32>::from(0u32),
                    stvec: U::<32>::from(0u32),
                    sscratch: U::<32>::from(0u32),
                    menv_fiom: Bit::Zero,
                    senv_fiom: Bit::Zero,
                    sepc: U::<32>::from(0u32),
                    scause: U::<32>::from(0u32),
                    stval: U::<32>::from(0u32),
                    satp: U::<32>::from(0u32),
                    counteren: U::<6>::from(0u8),
                },
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
                _ if dbg_take.to_bool() => { self.m_busy <= Bit::Zero },
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
                    // A load's value comes from the bus, so its slot
                    // keeps the address, which its fault reports.
                    self.wb_alu <= mux(is_load, addr, wval);
                    // An AMO's store goes to the physical address.
                    self.wb_pa <= mux(self.x_done, self.x_pa.get(), addr);
                    self.wb_f3 <= f3;
                    self.wb_lane <= lane;
                    // Only a load that went out waits for an answer, and
                    // only it can be refused: one that trapped in
                    // execute went nowhere (issue 1084).
                    // An exclusive `sc.w`'s result is its answer, so it is
                    // a load to whatever uses it next: that waits for it
                    // to retire (issue 1408).
                    self.wb_load <=
                        send_load | ld_loc | dc_try | (send_store & is_sc & xd);
                    self.wb_amo <= is_rmw & (send_load | ld_loc);
                    self.wb_x <= is_rmw & send_load & xd;
                    self.wb_scx <= send_store & is_sc & xd;
                    self.wb_loc <= ld_loc;
                    self.wb_dc <= dc_try;
                    // Writeback updates a line only at a DDR3 address the
                    // cache holds (`dc_in`, `dc_tag_hit`), so a store asks
                    // whatever its base register held (issue 1463).
                    self.wb_st <= store & !local & dc_on & !is_sc;
                    self.wb_sd <= sdata;
                    self.wb_en <= en;
                    self.amo_op <= funct5;
                    self.amo_b <= b;
                    self.wb_stop <= halting
                },
                _ if (self.dev_wait | amo_busy | scx_wait).to_bool() => {},
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
            self.redirect.set(
                (run & jump)
                    | int_take
                    | resume_take
                    | st_take
                    | wb_fault
                    | self.tp
                    | self.bp_fix,
            );
            let redirect = self.redirect.get();
            let park = run & stop;
            let hold = stall | (stop & !run) | in_debug;
            let zero = U::<32>::from(0u32);
            let width = mux(short, U::<32>::from(2u32), U::<32>::from(4u32));
            let advance = mux(hold, fetch_pc, fetch_pc + width);
            let go = mux(rst, zero, mux(park, link, advance));
            // An interrupt's or a refused store's handler, or the
            // instruction's own target.
            let late = mux(int_take | st_take, vec_other, target);
            let jmp = mux(
                rst,
                zero,
                mux(
                    park,
                    link,
                    mux(
                        resume_take,
                        dpc,
                        mux(
                            wb_fault,
                            wb_vec,
                            mux(
                                self.tp,
                                self.tp_vec.get(),
                                mux(self.bp_fix, self.bp_vec.get(), late),
                            ),
                        ),
                    ),
                ),
            );
            self.pc.set(mux(redirect, jmp, go));
            // The next instruction's register numbers, which the read
            // port and the forwarding's compares take a cycle early: the
            // fetched word's when it goes into the instruction register
            // below, else the one there now (issue 1130).
            let load_ir =
                !rst & !wb_fault & !stall & !stop & !(in_debug | dbg_take);
            let rs1_next = mux(load_ir, fetched.slice::<15, 5>(), rs1);
            let rs2_next = mux(load_ir, fetched.slice::<20, 5>(), rs2);
            // What the writeback stage will write: the instruction now in
            // execute if it goes on, else what is there, held or cleared;
            // a cleared one writes nothing, which `wb_here` says.
            let wr_next = mux(live, rd, wb_rd);
            // Whether it writes a register at all, from the decode alone:
            // an instruction that does not write, or writes x0, is never
            // forwarded (issue 1288). One that traps, or that an interrupt
            // takes the place of, writes nothing either, but the
            // instruction behind it is squashed, so what it would forward
            // is never used, and the late trap stays out of these compares.
            let now_any = writes & Bit::from(rd != 0);
            let wr_any = mux(live, now_any, Bit::from(wb_rd != 0));
            // What drops the registered interrupt decision (issue 1331):
            // an instruction that changes what may be taken, as it runs,
            // which is a write of one of the enables' registers or an
            // `mret` or `sret`; a trap or an interrupt taken, which
            // clears `MIE` or `SIE`; debug mode, its entry and its exit;
            // a refused load; and the reset. Only as it runs: a CSR
            // instruction that merely sits in execute, stalled or a read,
            // dropped it in every cycle, and a loop reading `mip` while
            // it waited for an interrupt never took one.
            let enables = Bit::from(f12 == isa::CSR_MSTATUS)
                | Bit::from(f12 == isa::CSR_SSTATUS)
                | Bit::from(f12 == isa::CSR_MIE)
                | Bit::from(f12 == isa::CSR_SIE)
                | Bit::from(f12 == isa::CSR_MIDELEG)
                | Bit::from(f12 == isa::CSR_MIP)
                | Bit::from(f12 == isa::CSR_SIP);
            let int_hold = rst
                | in_debug
                | dbg_take
                | resume_take
                | trap
                | wb_fault
                | (csr_write & enables)
                | mret_ok
                | sret_ok;
            with!(self <= {
                ra_at: mux(in_debug, dbg_gpr, rs1_next),
                m_a: Bit::from(wr_next == rs1_next) & wr_any,
                m_b: Bit::from(wr_next == rs2_next) & wr_any,
                csr_at: csr_index(mux(in_debug, dbg_csr, f12)),
                ir_new: load_ir,
                sc_q: sc_hit,
                sc_ready: self.valid & is_sc & stall & !stall_ld & !wb_fault,
                tp: exc,
                bp_fix: wrong,
                bp_vec: right_at,
                tp_vec: trap_vec,
                int_q: int_now & !int_hold,
                int_m_q: Bit::from(m_set != 0),
                int_set_q: int_set,
            });
            case!(rst => {
                Bit::One => { self.valid <= Bit::Zero },
                _ if wb_fault.to_bool() => { self.valid <= Bit::Zero },
                _ if stall.to_bool() => {},
                _ if stop.to_bool() => { self.valid <= Bit::Zero },
                _ if (in_debug | dbg_take).to_bool() => {
                    self.valid <= Bit::Zero
                },
                _ => {
                    self.ir <= fetched;
                    self.ir_bad <= f_fault;
                    self.ir_pf <= f_pf;
                    self.ir_pf2 <= !self.f_pf0;
                    self.ir_c <= Bit::from(short);
                    self.ir_pc <= fetch_pc;
                    // The word behind an exception is squashed here,
                    // and the redirect follows (issue 1195).
                    self.valid <= !redirect & !exc & !wrong
                },
            });
            // end{fetch}
            self.stopped.set(stop);
            halt.set(self.halted);
            dbg.set(self.debug);
            asleep.set(self.waiting.get());
            // What the debug module asked for: a general register through
            // the lent read port, `x0` as zero, or a CSR through the
            // CSR read, whose number is the module's in debug mode.
            dbg_rdata.set(mux(
                dbg_is_gpr,
                mux(dbg_gpr == 0, U::<32>::from(0u32), ra),
                csr_old,
            ));
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
