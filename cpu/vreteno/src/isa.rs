// SPDX-License-Identifier: Apache-2.0
//! RV32IMC as bits: the encoders a program is written with, and the
//! decoder the reference model reads with, which reads a compressed
//! instruction as the thirty-two bit one it stands for. The core
//! decodes and expands on its own, from the fields of the word, so the
//! two decoders check each other.

/// Every RV32IM instruction the core runs, by mnemonic.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Lui,
    Auipc,
    Jal,
    Jalr,
    Beq,
    Bne,
    Blt,
    Bge,
    Bltu,
    Bgeu,
    Lb,
    Lh,
    Lw,
    Lbu,
    Lhu,
    Sb,
    Sh,
    Sw,
    Addi,
    Slti,
    Sltiu,
    Xori,
    Ori,
    Andi,
    Slli,
    Srli,
    Srai,
    Add,
    Sub,
    Sll,
    Slt,
    Sltu,
    Xor,
    Srl,
    Sra,
    Or,
    And,
    Mul,
    Mulh,
    Mulhsu,
    Mulhu,
    Div,
    Divu,
    Rem,
    Remu,
    Fence,
    Ecall,
    Ebreak,
    Mret,
    Wfi,
    Csrrw,
    Csrrs,
    Csrrc,
    Csrrwi,
    Csrrsi,
    Csrrci,
    Illegal,
}

/// The control and status registers the core has: enough to take a
/// trap and return from it. The rest are illegal.
pub const CSR_MSTATUS: u32 = 0x300;
pub const CSR_MTVEC: u32 = 0x305;
pub const CSR_MSCRATCH: u32 = 0x340;
pub const CSR_MEPC: u32 = 0x341;
pub const CSR_MCAUSE: u32 = 0x342;
pub const CSR_MIE: u32 = 0x304;
pub const CSR_MIP: u32 = 0x344;
pub const CSR_MTVAL: u32 = 0x343;

/// What this core is, as `misa` reports it: `MXL` of 1 for a 32-bit
/// machine in the top two bits, and the letters `I`, `M` and `C` in
/// the extension bits, which are numbered from `A` at zero. So
/// RV32IMC, which is what the core implements.
pub const CSR_MISA: u32 = 0x301;
pub const MISA: u32 = 0x4000_0000 | (1 << 12) | (1 << 8) | (1 << 2);

/// The two machine counters, each 64 bits and each read as two
/// words. The specification makes them writable, so that software can
/// set a starting point, and this core allows that.
///
/// `mcycle` counts cycles and `minstret` counts instructions retired,
/// so the difference between them over a stretch is what the pipeline
/// spent on stalls: a multiply, a divide, a load from the bus, a
/// fetch above the boot memory, or a `wfi`.
pub const CSR_MCYCLE: u32 = 0xb00;
pub const CSR_MINSTRET: u32 = 0xb02;
pub const CSR_MCYCLEH: u32 = 0xb80;
pub const CSR_MINSTRETH: u32 = 0xb82;

/// The machine information registers. All four read as zero: a vendor
/// of zero means unassigned, an architecture and an implementation of
/// zero mean unspecified, and this machine has one hart, whose index
/// is zero.
///
/// Their addresses begin `0xf`, whose top two bits are ones, and the
/// privileged specification says a register numbered that way is read
/// only and that a write to one raises an illegal instruction. That
/// is what the core does, and it is why `misa` is not among them:
/// `misa` is writable and may ignore what is written, so a write to
/// it is legal and does nothing.
pub const CSR_MVENDORID: u32 = 0xf11;
pub const CSR_MARCHID: u32 = 0xf12;
pub const CSR_MIMPID: u32 = 0xf13;
pub const CSR_MHARTID: u32 = 0xf14;

/// The halt: a write of an odd value to this custom machine register
/// stops the core, and nothing restarts it. `csrwi 0x7c0, 1` is the
/// whole of it, one instruction, which is what a program says when it
/// is finished.
///
/// `ebreak` used to stop the core instead. It now raises the
/// breakpoint exception, which is what it is for and what a debugger
/// needs, so a program that means to stop says so here. That is
/// issue 139.
pub const CSR_MHALT: u32 = 0x7c0;

/// The debug control and status register, as the RISC-V debug
/// specification lays it out: `xdebugver` 4 in bits 31 to 28, `ebreakm`
/// bit 15, `cause` bits 8 to 6 (1 an `ebreak`, 3 a halt request, 4 a
/// step), `step` bit 2, and `prv` 3 in bits 1 and 0. A program or a
/// debugger writes `ebreakm` and `step`; the rest is what the core says.
pub const CSR_DCSR: u32 = 0x7b0;
/// The debug program counter: the instruction the core will execute on
/// resume, which is the one it did not execute when it entered debug
/// mode.
pub const CSR_DPC: u32 = 0x7b1;

/// The causes the core raises: five exceptions, and the external
/// interrupt, whose cause has the top bit set.
pub const CAUSE_ILLEGAL: u32 = 2;
/// `ebreak`, and anything else a debugger plants. The trap value is
/// the address of the instruction that raised it.
pub const CAUSE_BREAKPOINT: u32 = 3;
/// A load whose address is not a multiple of its width, and a store
/// of the same. The specification lets a core either support such an
/// access or raise these; this one raises them.
pub const CAUSE_LOAD_MISALIGNED: u32 = 4;
pub const CAUSE_STORE_MISALIGNED: u32 = 6;
pub const CAUSE_ECALL: u32 = 11;
pub const CAUSE_MEXT: u32 = 0x8000_000b;
pub const CAUSE_MTIMER: u32 = 0x8000_0007;
/// A software interrupt: what a program raises for itself by writing
/// `msip`, and what an operating system enters its scheduler with.
pub const CAUSE_MSOFT: u32 = 0x8000_0003;
/// The external, the timer and the software interrupt's bits in `mie`
/// and `mip`.
pub const MEXT: u32 = 1 << 11;
pub const MTIMER: u32 = 1 << 7;
pub const MSOFT: u32 = 1 << 3;
/// The core's interrupt controller, at the offsets every RISC-V
/// platform puts them at, so that a stock port of an operating system
/// finds them where it looks: `msip` first, the compare at `0x4000`
/// and the count at `0xbff8`. The window is 64 KiB, which is what
/// those offsets need.
pub const CLINT_BASE: u32 = 0x0200_0000;
pub const CLINT_MASK: u32 = 0xffff_0000;
/// A write of one to `msip` raises the software interrupt; a write of
/// zero clears it.
pub const MSIP_OFF: u32 = 0x0000;
/// The compare, low half then high.
pub const MTIMECMP_OFF: u32 = 0x4000;
/// The count, low half then high.
pub const MTIME_OFF: u32 = 0xbff8;
/// The serial port's two words: a byte to send, and the status, whose
/// bit 0 is busy.
pub const UART_BASE: u32 = 0x3000;

/// A decoded instruction: the mnemonic and its fields, the immediate
/// already extended to a signed word.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Decoded {
    pub kind: Kind,
    pub rd: u32,
    pub rs1: u32,
    pub rs2: u32,
    pub imm: i32,
}

pub const OP_LUI: u32 = 0x37;
pub const OP_AUIPC: u32 = 0x17;
pub const OP_JAL: u32 = 0x6f;
pub const OP_JALR: u32 = 0x67;
pub const OP_BRANCH: u32 = 0x63;
pub const OP_LOAD: u32 = 0x03;
pub const OP_STORE: u32 = 0x23;
pub const OP_IMM: u32 = 0x13;
pub const OP_OP: u32 = 0x33;
pub const OP_FENCE: u32 = 0x0f;
pub const OP_SYSTEM: u32 = 0x73;

// The six formats.
fn r(op: u32, rd: u32, f3: u32, rs1: u32, rs2: u32, f7: u32) -> u32 {
    (f7 << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op
}
fn i(op: u32, rd: u32, f3: u32, rs1: u32, imm: i32) -> u32 {
    ((imm as u32 & 0xfff) << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op
}
fn s(op: u32, f3: u32, rs1: u32, rs2: u32, imm: i32) -> u32 {
    let imm = imm as u32;
    ((imm >> 5 & 0x7f) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (f3 << 12)
        | ((imm & 0x1f) << 7)
        | op
}
fn b(op: u32, f3: u32, rs1: u32, rs2: u32, imm: i32) -> u32 {
    let imm = imm as u32;
    ((imm >> 12 & 1) << 31)
        | ((imm >> 5 & 0x3f) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (f3 << 12)
        | ((imm >> 1 & 0xf) << 8)
        | ((imm >> 11 & 1) << 7)
        | op
}
fn u(op: u32, rd: u32, imm: i32) -> u32 {
    (imm as u32 & 0xfffff000) | (rd << 7) | op
}
fn j(op: u32, rd: u32, imm: i32) -> u32 {
    let imm = imm as u32;
    ((imm >> 20 & 1) << 31)
        | ((imm >> 1 & 0x3ff) << 21)
        | ((imm >> 11 & 1) << 20)
        | ((imm >> 12 & 0xff) << 12)
        | (rd << 7)
        | op
}

// The encoders, named as the assembler names them. Immediates are
// what the assembler takes: bytes for branches and jumps, the value
// for the rest, and the upper twenty bits for `lui` and `auipc`.
pub fn lui(rd: u32, imm20: u32) -> u32 {
    u(OP_LUI, rd, (imm20 << 12) as i32)
}
pub fn auipc(rd: u32, imm20: u32) -> u32 {
    u(OP_AUIPC, rd, (imm20 << 12) as i32)
}
pub fn jal(rd: u32, off: i32) -> u32 {
    j(OP_JAL, rd, off)
}
pub fn jalr(rd: u32, rs1: u32, off: i32) -> u32 {
    i(OP_JALR, rd, 0, rs1, off)
}
pub fn beq(rs1: u32, rs2: u32, off: i32) -> u32 {
    b(OP_BRANCH, 0, rs1, rs2, off)
}
pub fn bne(rs1: u32, rs2: u32, off: i32) -> u32 {
    b(OP_BRANCH, 1, rs1, rs2, off)
}
pub fn blt(rs1: u32, rs2: u32, off: i32) -> u32 {
    b(OP_BRANCH, 4, rs1, rs2, off)
}
pub fn bge(rs1: u32, rs2: u32, off: i32) -> u32 {
    b(OP_BRANCH, 5, rs1, rs2, off)
}
pub fn bltu(rs1: u32, rs2: u32, off: i32) -> u32 {
    b(OP_BRANCH, 6, rs1, rs2, off)
}
pub fn bgeu(rs1: u32, rs2: u32, off: i32) -> u32 {
    b(OP_BRANCH, 7, rs1, rs2, off)
}
pub fn lb(rd: u32, rs1: u32, off: i32) -> u32 {
    i(OP_LOAD, rd, 0, rs1, off)
}
pub fn lh(rd: u32, rs1: u32, off: i32) -> u32 {
    i(OP_LOAD, rd, 1, rs1, off)
}
pub fn lw(rd: u32, rs1: u32, off: i32) -> u32 {
    i(OP_LOAD, rd, 2, rs1, off)
}
pub fn lbu(rd: u32, rs1: u32, off: i32) -> u32 {
    i(OP_LOAD, rd, 4, rs1, off)
}
pub fn lhu(rd: u32, rs1: u32, off: i32) -> u32 {
    i(OP_LOAD, rd, 5, rs1, off)
}
pub fn sb(rs2: u32, rs1: u32, off: i32) -> u32 {
    s(OP_STORE, 0, rs1, rs2, off)
}
pub fn sh(rs2: u32, rs1: u32, off: i32) -> u32 {
    s(OP_STORE, 1, rs1, rs2, off)
}
pub fn sw(rs2: u32, rs1: u32, off: i32) -> u32 {
    s(OP_STORE, 2, rs1, rs2, off)
}
pub fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    i(OP_IMM, rd, 0, rs1, imm)
}
pub fn slti(rd: u32, rs1: u32, imm: i32) -> u32 {
    i(OP_IMM, rd, 2, rs1, imm)
}
pub fn sltiu(rd: u32, rs1: u32, imm: i32) -> u32 {
    i(OP_IMM, rd, 3, rs1, imm)
}
pub fn xori(rd: u32, rs1: u32, imm: i32) -> u32 {
    i(OP_IMM, rd, 4, rs1, imm)
}
pub fn ori(rd: u32, rs1: u32, imm: i32) -> u32 {
    i(OP_IMM, rd, 6, rs1, imm)
}
pub fn andi(rd: u32, rs1: u32, imm: i32) -> u32 {
    i(OP_IMM, rd, 7, rs1, imm)
}
pub fn slli(rd: u32, rs1: u32, sh: u32) -> u32 {
    r(OP_IMM, rd, 1, rs1, sh, 0)
}
pub fn srli(rd: u32, rs1: u32, sh: u32) -> u32 {
    r(OP_IMM, rd, 5, rs1, sh, 0)
}
pub fn srai(rd: u32, rs1: u32, sh: u32) -> u32 {
    r(OP_IMM, rd, 5, rs1, sh, 0x20)
}
pub fn add(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 0, rs1, rs2, 0)
}
pub fn sub(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 0, rs1, rs2, 0x20)
}
pub fn sll(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 1, rs1, rs2, 0)
}
pub fn slt(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 2, rs1, rs2, 0)
}
pub fn sltu(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 3, rs1, rs2, 0)
}
pub fn xor(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 4, rs1, rs2, 0)
}
pub fn srl(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 5, rs1, rs2, 0)
}
pub fn sra(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 5, rs1, rs2, 0x20)
}
pub fn or(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 6, rs1, rs2, 0)
}
pub fn and(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 7, rs1, rs2, 0)
}
// The M extension: the same format, funct7 = 1.
pub fn mul(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 0, rs1, rs2, 1)
}
pub fn mulh(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 1, rs1, rs2, 1)
}
pub fn mulhsu(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 2, rs1, rs2, 1)
}
pub fn mulhu(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 3, rs1, rs2, 1)
}
pub fn div(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 4, rs1, rs2, 1)
}
pub fn divu(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 5, rs1, rs2, 1)
}
pub fn rem(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 6, rs1, rs2, 1)
}
pub fn remu(rd: u32, rs1: u32, rs2: u32) -> u32 {
    r(OP_OP, rd, 7, rs1, rs2, 1)
}
pub fn fence() -> u32 {
    i(OP_FENCE, 0, 0, 0, 0)
}
pub fn ecall() -> u32 {
    i(OP_SYSTEM, 0, 0, 0, 0)
}
pub fn ebreak() -> u32 {
    i(OP_SYSTEM, 0, 0, 0, 1)
}
pub fn mret() -> u32 {
    i(OP_SYSTEM, 0, 0, 0, 0x302)
}
/// Wait for an interrupt: the core stops fetching until one is
/// pending and enabled, whether or not interrupts are enabled
/// globally, which is what lets a kernel idle inside its own lock.
pub fn wfi() -> u32 {
    i(OP_SYSTEM, 0, 0, 0, 0x105)
}
/// What a program says when it is finished: a write of one to
/// `CSR_MHALT`, in one instruction. `ebreak` used to do this and now
/// raises the breakpoint exception instead, which is issue 139.
pub fn halt() -> u32 {
    csrrwi(0, CSR_MHALT, 1)
}
// The CSR instructions: a register form and an immediate form of each
// of write, set and clear; `rd` takes the old value.
pub fn csrrw(rd: u32, csr: u32, rs1: u32) -> u32 {
    i(OP_SYSTEM, rd, 1, rs1, csr as i32)
}
pub fn csrrs(rd: u32, csr: u32, rs1: u32) -> u32 {
    i(OP_SYSTEM, rd, 2, rs1, csr as i32)
}
pub fn csrrc(rd: u32, csr: u32, rs1: u32) -> u32 {
    i(OP_SYSTEM, rd, 3, rs1, csr as i32)
}
pub fn csrrwi(rd: u32, csr: u32, zimm: u32) -> u32 {
    i(OP_SYSTEM, rd, 5, zimm, csr as i32)
}
pub fn csrrsi(rd: u32, csr: u32, zimm: u32) -> u32 {
    i(OP_SYSTEM, rd, 6, zimm, csr as i32)
}
pub fn csrrci(rd: u32, csr: u32, zimm: u32) -> u32 {
    i(OP_SYSTEM, rd, 7, zimm, csr as i32)
}

// The compressed instructions, RV32C: sixteen bits each, every one of
// them a shorter spelling of an instruction above. A register field of
// three bits names x8 to x15, the registers the calling convention uses
// most. The encoders take register numbers and immediates as the
// assembler does, and are named as it names them.

/// Whether a halfword starts a compressed instruction: the low two
/// bits of every thirty-two bit instruction are both set.
pub fn is_compressed(half: u16) -> bool {
    half & 3 != 3
}

/// A register of x8 to x15, as the three bits that name it.
fn creg(r: u32) -> u32 {
    assert!((8..16).contains(&r), "x{r} has no three-bit name");
    r - 8
}

fn c(v: u32) -> u16 {
    v as u16
}

pub fn c_nop() -> u16 {
    0x0001
}
pub fn c_addi(rd: u32, imm: i32) -> u16 {
    let imm = imm as u32;
    c((imm >> 5 & 1) << 12 | rd << 7 | (imm & 0x1f) << 2 | 1)
}
pub fn c_li(rd: u32, imm: i32) -> u16 {
    let imm = imm as u32;
    c(2 << 13 | (imm >> 5 & 1) << 12 | rd << 7 | (imm & 0x1f) << 2 | 1)
}
/// `lui rd, imm`, where `imm` is the upper value, -32 to 31, not 0.
pub fn c_lui(rd: u32, imm: i32) -> u16 {
    let imm = imm as u32;
    c(3 << 13 | (imm >> 5 & 1) << 12 | rd << 7 | (imm & 0x1f) << 2 | 1)
}
/// `addi sp, sp, imm`, `imm` a multiple of 16 from -512 to 496.
pub fn c_addi16sp(imm: i32) -> u16 {
    let i = imm as u32;
    c(3 << 13
        | (i >> 9 & 1) << 12
        | 2 << 7
        | (i >> 4 & 1) << 6
        | (i >> 6 & 1) << 5
        | (i >> 7 & 3) << 3
        | (i >> 5 & 1) << 2
        | 1)
}
/// `addi rd, sp, imm`, `imm` a multiple of 4 from 4 to 1020.
pub fn c_addi4spn(rd: u32, imm: u32) -> u16 {
    c((imm >> 4 & 3) << 11
        | (imm >> 6 & 0xf) << 7
        | (imm >> 2 & 1) << 6
        | (imm >> 3 & 1) << 5
        | creg(rd) << 2)
}
pub fn c_slli(rd: u32, sh: u32) -> u16 {
    c((sh >> 5 & 1) << 12 | rd << 7 | (sh & 0x1f) << 2 | 2)
}
pub fn c_srli(rd: u32, sh: u32) -> u16 {
    c(4 << 13 | (sh >> 5 & 1) << 12 | creg(rd) << 7 | (sh & 0x1f) << 2 | 1)
}
pub fn c_srai(rd: u32, sh: u32) -> u16 {
    c(4 << 13
        | (sh >> 5 & 1) << 12
        | 1 << 10
        | creg(rd) << 7
        | (sh & 0x1f) << 2
        | 1)
}
pub fn c_andi(rd: u32, imm: i32) -> u16 {
    let imm = imm as u32;
    c(4 << 13
        | (imm >> 5 & 1) << 12
        | 2 << 10
        | creg(rd) << 7
        | (imm & 0x1f) << 2
        | 1)
}
/// The four register operations of the arithmetic group: `f` is 0 for
/// sub, 1 xor, 2 or, 3 and.
fn c_arith(f: u32, rd: u32, rs2: u32) -> u16 {
    c(4 << 13 | 3 << 10 | creg(rd) << 7 | f << 5 | creg(rs2) << 2 | 1)
}
pub fn c_sub(rd: u32, rs2: u32) -> u16 {
    c_arith(0, rd, rs2)
}
pub fn c_xor(rd: u32, rs2: u32) -> u16 {
    c_arith(1, rd, rs2)
}
pub fn c_or(rd: u32, rs2: u32) -> u16 {
    c_arith(2, rd, rs2)
}
pub fn c_and(rd: u32, rs2: u32) -> u16 {
    c_arith(3, rd, rs2)
}
/// A jump's offset as `c.j` and `c.jal` scatter it.
fn cj(off: i32) -> u32 {
    let o = off as u32;
    (o >> 11 & 1) << 12
        | (o >> 4 & 1) << 11
        | (o >> 8 & 3) << 9
        | (o >> 10 & 1) << 8
        | (o >> 6 & 1) << 7
        | (o >> 7 & 1) << 6
        | (o >> 1 & 7) << 3
        | (o >> 5 & 1) << 2
}
pub fn c_j(off: i32) -> u16 {
    c(5 << 13 | cj(off) | 1)
}
pub fn c_jal(off: i32) -> u16 {
    c(1 << 13 | cj(off) | 1)
}
/// A branch's offset as `c.beqz` and `c.bnez` scatter it.
fn cb(off: i32) -> u32 {
    let o = off as u32;
    (o >> 8 & 1) << 12
        | (o >> 3 & 3) << 10
        | (o >> 6 & 3) << 5
        | (o >> 1 & 3) << 3
        | (o >> 5 & 1) << 2
}
pub fn c_beqz(rs1: u32, off: i32) -> u16 {
    c(6 << 13 | cb(off) | creg(rs1) << 7 | 1)
}
pub fn c_bnez(rs1: u32, off: i32) -> u16 {
    c(7 << 13 | cb(off) | creg(rs1) << 7 | 1)
}
pub fn c_lw(rd: u32, rs1: u32, off: u32) -> u16 {
    c(2 << 13
        | (off >> 3 & 7) << 10
        | creg(rs1) << 7
        | (off >> 2 & 1) << 6
        | (off >> 6 & 1) << 5
        | creg(rd) << 2)
}
pub fn c_sw(rs2: u32, rs1: u32, off: u32) -> u16 {
    c(6 << 13
        | (off >> 3 & 7) << 10
        | creg(rs1) << 7
        | (off >> 2 & 1) << 6
        | (off >> 6 & 1) << 5
        | creg(rs2) << 2)
}
/// `lw rd, off(sp)`, `off` a multiple of 4 from 0 to 252.
pub fn c_lwsp(rd: u32, off: u32) -> u16 {
    c(2 << 13
        | (off >> 5 & 1) << 12
        | rd << 7
        | (off >> 2 & 7) << 4
        | (off >> 6 & 3) << 2
        | 2)
}
/// `sw rs2, off(sp)`, `off` a multiple of 4 from 0 to 252.
pub fn c_swsp(rs2: u32, off: u32) -> u16 {
    c(6 << 13 | (off >> 2 & 0xf) << 9 | (off >> 6 & 3) << 7 | rs2 << 2 | 2)
}
pub fn c_jr(rs1: u32) -> u16 {
    c(4 << 13 | rs1 << 7 | 2)
}
pub fn c_mv(rd: u32, rs2: u32) -> u16 {
    c(4 << 13 | rd << 7 | rs2 << 2 | 2)
}
pub fn c_ebreak() -> u16 {
    0x9002
}
pub fn c_jalr(rs1: u32) -> u16 {
    c(4 << 13 | 1 << 12 | rs1 << 7 | 2)
}
pub fn c_add(rd: u32, rs2: u32) -> u16 {
    c(4 << 13 | 1 << 12 | rd << 7 | rs2 << 2 | 2)
}

/// A compressed instruction as the thirty-two bit instruction it
/// stands for, or `None` for a halfword that is no RV32C instruction:
/// a reserved pattern, a floating-point or 64-bit one, or a shift by
/// 32 or more, which RV32C leaves to custom extensions. A hint, such
/// as `c.addi` of zero or `c.mv` into x0, is an instruction, and
/// expands to the one it is spelt as, which does nothing.
pub fn compressed(h: u16) -> Option<u32> {
    let h = h as u32;
    let bit = |i: u32| h >> i & 1;
    let bits = |hi: u32, lo: u32| h >> lo & ((1 << (hi - lo + 1)) - 1);
    let sext =
        |v: u32, width: u32| ((v << (32 - width)) as i32) >> (32 - width);
    let rd = bits(11, 7);
    let rs2 = bits(6, 2);
    // The three-bit register fields, as register numbers.
    let rs1s = bits(9, 7) + 8;
    let rs2s = bits(4, 2) + 8;
    // The six-bit immediate of c.addi, c.li, c.andi and the shifts.
    let imm6 = sext(bit(12) << 5 | bits(6, 2), 6);
    let sh = bit(12) << 5 | bits(6, 2);
    let jimm = sext(
        bit(12) << 11
            | bit(8) << 10
            | bits(10, 9) << 8
            | bit(6) << 7
            | bit(7) << 6
            | bit(2) << 5
            | bit(11) << 4
            | bits(5, 3) << 1,
        12,
    );
    let bimm = sext(
        bit(12) << 8
            | bits(6, 5) << 6
            | bit(2) << 5
            | bits(11, 10) << 3
            | bits(4, 3) << 1,
        9,
    );
    let lwimm = (bit(5) << 6 | bits(12, 10) << 3 | bit(6) << 2) as i32;
    Some(match (bits(1, 0), bits(15, 13)) {
        (0, 0) => {
            let imm = bits(10, 7) << 6
                | bits(12, 11) << 4
                | bit(5) << 3
                | bit(6) << 2;
            if imm == 0 {
                return None;
            }
            addi(rs2s, 2, imm as i32)
        }
        (0, 2) => lw(rs2s, rs1s, lwimm),
        (0, 6) => sw(rs2s, rs1s, lwimm),
        (1, 0) => addi(rd, rd, imm6),
        (1, 1) => jal(1, jimm),
        (1, 2) => addi(rd, 0, imm6),
        (1, 3) => {
            if bit(12) == 0 && bits(6, 2) == 0 {
                return None;
            }
            if rd == 2 {
                let imm = bit(12) << 9
                    | bits(4, 3) << 7
                    | bit(5) << 6
                    | bit(2) << 5
                    | bit(6) << 4;
                addi(2, 2, sext(imm, 10))
            } else {
                lui(rd, imm6 as u32 & 0xfffff)
            }
        }
        (1, 4) => match bits(11, 10) {
            0 if bit(12) == 0 => srli(rs1s, rs1s, sh),
            1 if bit(12) == 0 => srai(rs1s, rs1s, sh),
            2 => andi(rs1s, rs1s, imm6),
            3 if bit(12) == 0 => match bits(6, 5) {
                0 => sub(rs1s, rs1s, rs2s),
                1 => xor(rs1s, rs1s, rs2s),
                2 => or(rs1s, rs1s, rs2s),
                _ => and(rs1s, rs1s, rs2s),
            },
            _ => return None,
        },
        (1, 5) => jal(0, jimm),
        (1, 6) => beq(rs1s, 0, bimm),
        (1, 7) => bne(rs1s, 0, bimm),
        (2, 0) if bit(12) == 0 => slli(rd, rd, sh),
        (2, 2) if rd != 0 => lw(
            rd,
            2,
            (bits(3, 2) << 6 | bit(12) << 5 | bits(6, 4) << 2) as i32,
        ),
        (2, 4) => match (bit(12), rd, rs2) {
            (0, 0, 0) => return None,
            (0, _, 0) => jalr(0, rd, 0),
            (0, _, _) => add(rd, 0, rs2),
            (_, 0, 0) => ebreak(),
            (_, _, 0) => jalr(1, rd, 0),
            _ => add(rd, rd, rs2),
        },
        (2, 6) => sw(rs2, 2, (bits(8, 7) << 6 | bits(12, 9) << 2) as i32),
        _ => return None,
    })
}

/// The compressed spelling of a thirty-two bit instruction, if it has
/// one: what an assembler does when it is allowed to. Where two
/// spellings fit, the one the list below reaches first is taken; each
/// expands back to `w`.
pub fn compress(w: u32) -> Option<u16> {
    let d = decode(w);
    let (rd, rs1, rs2, imm) = (d.rd, d.rs1, d.rs2, d.imm);
    let short = |r: u32| (8..16).contains(&r);
    let six = |v: i32| (-32..32).contains(&v);
    use Kind::*;
    Some(match d.kind {
        Addi if rd == 0 && rs1 == 0 && imm == 0 => c_nop(),
        Addi if rd == rs1 && rd != 0 && imm != 0 && six(imm) => c_addi(rd, imm),
        Addi if rs1 == 0 && rd != 0 && six(imm) => c_li(rd, imm),
        Addi if rd == 2 && rs1 == 2 && imm != 0 && imm % 16 == 0 => {
            if !(-512..512).contains(&imm) {
                return None;
            }
            c_addi16sp(imm)
        }
        Addi if rs1 == 2
            && short(rd)
            && imm > 0
            && imm < 1024
            && imm % 4 == 0 =>
        {
            c_addi4spn(rd, imm as u32)
        }
        Lui if rd != 0 && rd != 2 && imm != 0 && six(imm >> 12) => {
            c_lui(rd, imm >> 12)
        }
        Slli if rd == rs1 && rd != 0 => c_slli(rd, imm as u32),
        Srli if rd == rs1 && short(rd) => c_srli(rd, imm as u32),
        Srai if rd == rs1 && short(rd) => c_srai(rd, imm as u32),
        Andi if rd == rs1 && short(rd) && six(imm) => c_andi(rd, imm),
        Sub if rd == rs1 && short(rd) && short(rs2) => c_sub(rd, rs2),
        Xor if rd == rs1 && short(rd) && short(rs2) => c_xor(rd, rs2),
        Or if rd == rs1 && short(rd) && short(rs2) => c_or(rd, rs2),
        And if rd == rs1 && short(rd) && short(rs2) => c_and(rd, rs2),
        Add if rd == rs1 && rd != 0 && rs2 != 0 => c_add(rd, rs2),
        Add if rs1 == 0 && rd != 0 && rs2 != 0 => c_mv(rd, rs2),
        Jal if rd == 0 && (-2048..2048).contains(&imm) => c_j(imm),
        Jal if rd == 1 && (-2048..2048).contains(&imm) => c_jal(imm),
        Jalr if imm == 0 && rs1 != 0 && rd == 0 => c_jr(rs1),
        Jalr if imm == 0 && rs1 != 0 && rd == 1 => c_jalr(rs1),
        Beq if rs2 == 0 && short(rs1) && (-256..256).contains(&imm) => {
            c_beqz(rs1, imm)
        }
        Bne if rs2 == 0 && short(rs1) && (-256..256).contains(&imm) => {
            c_bnez(rs1, imm)
        }
        Lw if rs1 == 2
            && rd != 0
            && (0..256).contains(&imm)
            && imm % 4 == 0 =>
        {
            c_lwsp(rd, imm as u32)
        }
        Lw if short(rs1)
            && short(rd)
            && (0..128).contains(&imm)
            && imm % 4 == 0 =>
        {
            c_lw(rd, rs1, imm as u32)
        }
        Sw if rs1 == 2 && (0..256).contains(&imm) && imm % 4 == 0 => {
            c_swsp(rs2, imm as u32)
        }
        Sw if short(rs1)
            && short(rs2)
            && (0..128).contains(&imm)
            && imm % 4 == 0 =>
        {
            c_sw(rs2, rs1, imm as u32)
        }
        Ebreak => c_ebreak(),
        _ => return None,
    })
}

/// The fields of a word, by the format its opcode names.
pub fn decode(w: u32) -> Decoded {
    let op = w & 0x7f;
    let rd = w >> 7 & 0x1f;
    let f3 = w >> 12 & 7;
    let rs1 = w >> 15 & 0x1f;
    let rs2 = w >> 20 & 0x1f;
    let f7 = w >> 25;
    let imm_i = (w as i32) >> 20;
    let imm_s = ((w as i32) >> 25 << 5) | (w >> 7 & 0x1f) as i32;
    let imm_b = ((w as i32) >> 31 << 12)
        | ((w >> 7 & 1) << 11) as i32
        | ((w >> 25 & 0x3f) << 5) as i32
        | ((w >> 8 & 0xf) << 1) as i32;
    let imm_u = (w & 0xfffff000) as i32;
    let imm_j = ((w as i32) >> 31 << 20)
        | (w & 0xff000) as i32
        | ((w >> 20 & 1) << 11) as i32
        | ((w >> 21 & 0x3ff) << 1) as i32;
    let d = |kind, imm| Decoded {
        kind,
        rd,
        rs1,
        rs2,
        imm,
    };
    use Kind::*;
    match op {
        OP_LUI => d(Lui, imm_u),
        OP_AUIPC => d(Auipc, imm_u),
        OP_JAL => d(Jal, imm_j),
        OP_JALR if f3 == 0 => d(Jalr, imm_i),
        OP_BRANCH => match f3 {
            0 => d(Beq, imm_b),
            1 => d(Bne, imm_b),
            4 => d(Blt, imm_b),
            5 => d(Bge, imm_b),
            6 => d(Bltu, imm_b),
            7 => d(Bgeu, imm_b),
            _ => d(Illegal, 0),
        },
        OP_LOAD => match f3 {
            0 => d(Lb, imm_i),
            1 => d(Lh, imm_i),
            2 => d(Lw, imm_i),
            4 => d(Lbu, imm_i),
            5 => d(Lhu, imm_i),
            _ => d(Illegal, 0),
        },
        OP_STORE => match f3 {
            0 => d(Sb, imm_s),
            1 => d(Sh, imm_s),
            2 => d(Sw, imm_s),
            _ => d(Illegal, 0),
        },
        OP_IMM => match (f3, f7) {
            (0, _) => d(Addi, imm_i),
            (2, _) => d(Slti, imm_i),
            (3, _) => d(Sltiu, imm_i),
            (4, _) => d(Xori, imm_i),
            (6, _) => d(Ori, imm_i),
            (7, _) => d(Andi, imm_i),
            (1, 0) => d(Slli, rs2 as i32),
            (5, 0) => d(Srli, rs2 as i32),
            (5, 0x20) => d(Srai, rs2 as i32),
            _ => d(Illegal, 0),
        },
        OP_OP => match (f3, f7) {
            (0, 0) => d(Add, 0),
            (0, 0x20) => d(Sub, 0),
            (1, 0) => d(Sll, 0),
            (2, 0) => d(Slt, 0),
            (3, 0) => d(Sltu, 0),
            (4, 0) => d(Xor, 0),
            (5, 0) => d(Srl, 0),
            (5, 0x20) => d(Sra, 0),
            (6, 0) => d(Or, 0),
            (7, 0) => d(And, 0),
            (0, 1) => d(Mul, 0),
            (1, 1) => d(Mulh, 0),
            (2, 1) => d(Mulhsu, 0),
            (3, 1) => d(Mulhu, 0),
            (4, 1) => d(Div, 0),
            (5, 1) => d(Divu, 0),
            (6, 1) => d(Rem, 0),
            (7, 1) => d(Remu, 0),
            _ => d(Illegal, 0),
        },
        OP_FENCE => d(Fence, 0),
        // The system instructions: the immediate is the CSR address,
        // and an immediate form's operand sits in the rs1 field.
        OP_SYSTEM => match (f3, w >> 20) {
            (0, 0) => d(Ecall, 0),
            (0, 1) => d(Ebreak, 0),
            (0, 0x302) => d(Mret, 0),
            (0, 0x105) => d(Wfi, 0),
            (1, csr) => d(Csrrw, csr as i32),
            (2, csr) => d(Csrrs, csr as i32),
            (3, csr) => d(Csrrc, csr as i32),
            (5, csr) => d(Csrrwi, csr as i32),
            (6, csr) => d(Csrrsi, csr as i32),
            (7, csr) => d(Csrrci, csr as i32),
            _ => d(Illegal, 0),
        },
        _ => d(Illegal, 0),
    }
}

/// The instruction as assembler text, for a trace a person reads.
pub fn disasm(w: u32) -> String {
    let Decoded {
        kind,
        rd,
        rs1,
        rs2,
        imm,
    } = decode(w);
    let m = format!("{kind:?}").to_lowercase();
    use Kind::*;
    match kind {
        Lui | Auipc => format!("{m} x{rd}, {:#x}", (imm as u32) >> 12),
        Jal => format!("{m} x{rd}, {imm}"),
        Jalr | Lb | Lh | Lw | Lbu | Lhu => {
            format!("{m} x{rd}, {imm}(x{rs1})")
        }
        Beq | Bne | Blt | Bge | Bltu | Bgeu => {
            format!("{m} x{rs1}, x{rs2}, {imm}")
        }
        Sb | Sh | Sw => format!("{m} x{rs2}, {imm}(x{rs1})"),
        Addi | Slti | Sltiu | Xori | Ori | Andi | Slli | Srli | Srai => {
            format!("{m} x{rd}, x{rs1}, {imm}")
        }
        Add | Sub | Sll | Slt | Sltu | Xor | Srl | Sra | Or | And | Mul
        | Mulh | Mulhsu | Mulhu | Div | Divu | Rem | Remu => {
            format!("{m} x{rd}, x{rs1}, x{rs2}")
        }
        Fence | Ecall | Ebreak | Mret | Wfi => m,
        Csrrw | Csrrs | Csrrc => format!("{m} x{rd}, {imm:#x}, x{rs1}"),
        Csrrwi | Csrrsi | Csrrci => format!("{m} x{rd}, {imm:#x}, {rs1}"),
        Illegal => format!("illegal {w:#010x}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every encoder decodes to itself: the mnemonic, the immediate,
    /// and the registers its format has. A field a format does not
    /// have holds bits of the immediate, and is not compared.
    #[test]
    fn round_trip() {
        let cases = [
            (addi(1, 2, -5), Kind::Addi, Some(1), Some(2), None, -5),
            (lui(3, 0xfffff), Kind::Lui, Some(3), None, None, -4096),
            (jal(1, -8), Kind::Jal, Some(1), None, None, -8),
            (beq(4, 5, 12), Kind::Beq, None, Some(4), Some(5), 12),
            (bge(4, 5, -4096), Kind::Bge, None, Some(4), Some(5), -4096),
            (sw(7, 2, -12), Kind::Sw, None, Some(2), Some(7), -12),
            (lh(9, 2, 6), Kind::Lh, Some(9), Some(2), None, 6),
            (srai(1, 1, 31), Kind::Srai, Some(1), Some(1), None, 31),
            (sra(1, 2, 3), Kind::Sra, Some(1), Some(2), Some(3), 0),
            (mul(4, 5, 6), Kind::Mul, Some(4), Some(5), Some(6), 0),
            (mulhsu(4, 5, 6), Kind::Mulhsu, Some(4), Some(5), Some(6), 0),
            (remu(4, 5, 6), Kind::Remu, Some(4), Some(5), Some(6), 0),
            (ebreak(), Kind::Ebreak, None, None, None, 0),
            (mret(), Kind::Mret, None, None, None, 0),
            (
                csrrw(5, CSR_MEPC, 6),
                Kind::Csrrw,
                Some(5),
                Some(6),
                None,
                0x341,
            ),
            (
                csrrsi(0, CSR_MSTATUS, 8),
                Kind::Csrrsi,
                Some(0),
                Some(8),
                None,
                0x300,
            ),
        ];
        for (w, kind, rd, rs1, rs2, imm) in cases {
            let d = decode(w);
            let t = disasm(w);
            assert_eq!(d.kind, kind, "{t}");
            assert_eq!(d.imm, imm, "{t}");
            assert_eq!(rd.map(|_| d.rd), rd, "{t}");
            assert_eq!(rs1.map(|_| d.rs1), rs1, "{t}");
            assert_eq!(rs2.map(|_| d.rs2), rs2, "{t}");
        }
    }

    /// Every compressed encoder expands to the instruction it spells,
    /// at the ends of each immediate's range. The halfwords written out
    /// are as the GNU disassembler reads them.
    #[test]
    fn compressed_round_trip() {
        let cases = [
            (0x0505, addi(10, 10, 1)),
            (0x4515, addi(10, 0, 5)),
            (0x8082, jalr(0, 1, 0)),
            (0x852e, add(10, 0, 11)),
            (0x952e, add(10, 10, 11)),
            (0x9002, ebreak()),
            (0x0001, addi(0, 0, 0)),
            (0x0028, addi(10, 2, 8)),
            (0x1101, addi(2, 2, -32)),
            (0xc606, sw(1, 2, 12)),
            (0x40b2, lw(1, 2, 12)),
            (c_addi(31, -32), addi(31, 31, -32)),
            (c_addi(1, 31), addi(1, 1, 31)),
            (c_li(5, -1), addi(5, 0, -1)),
            (c_lui(7, -32), lui(7, 0xfffe0)),
            (c_lui(7, 31), lui(7, 31)),
            (c_addi16sp(-512), addi(2, 2, -512)),
            (c_addi16sp(496), addi(2, 2, 496)),
            (c_addi4spn(15, 1020), addi(15, 2, 1020)),
            (c_addi4spn(8, 4), addi(8, 2, 4)),
            (c_slli(3, 31), slli(3, 3, 31)),
            (c_srli(9, 1), srli(9, 9, 1)),
            (c_srai(10, 31), srai(10, 10, 31)),
            (c_andi(11, -32), andi(11, 11, -32)),
            (c_sub(12, 13), sub(12, 12, 13)),
            (c_xor(14, 15), xor(14, 14, 15)),
            (c_or(8, 9), or(8, 8, 9)),
            (c_and(10, 11), and(10, 10, 11)),
            (c_j(-2048), jal(0, -2048)),
            (c_j(2046), jal(0, 2046)),
            (c_jal(-2), jal(1, -2)),
            (c_beqz(8, -256), beq(8, 0, -256)),
            (c_bnez(15, 254), bne(15, 0, 254)),
            (c_lw(8, 15, 124), lw(8, 15, 124)),
            (c_sw(9, 14, 4), sw(9, 14, 4)),
            (c_lwsp(31, 252), lw(31, 2, 252)),
            (c_swsp(4, 0), sw(4, 2, 0)),
            (c_jr(5), jalr(0, 5, 0)),
            (c_jalr(6), jalr(1, 6, 0)),
            (c_mv(7, 8), add(7, 0, 8)),
            (c_add(9, 10), add(9, 9, 10)),
            (c_ebreak(), ebreak()),
            (c_nop(), addi(0, 0, 0)),
        ];
        for (h, w) in cases {
            assert!(is_compressed(h), "{h:#06x} is not compressed");
            assert_eq!(compressed(h), Some(w), "{h:#06x}: {}", disasm(w));
        }
        // Reserved: all zeros, c.jr of x0, c.lwsp into x0, c.lui and
        // c.addi16sp of zero, a shift by 32, and c.addiw, which is
        // RV64's.
        for h in [0x0000, 0x8002, 0x4002, 0x6081, 0x6101, 0x1082, 0x9c01] {
            assert_eq!(compressed(h), None, "{h:#06x} should be reserved");
        }
    }

    /// Compressing an instruction and expanding it again gives the
    /// instruction back, for every instruction a compressed halfword
    /// stands for; and every one of those but the hints, the ones that
    /// write x0 and the additions of zero, finds a compressed spelling.
    #[test]
    fn compress_inverts_compressed() {
        let mut found = 0;
        let mut spelt = 0;
        for h in (0u32..0x10000).filter(|h| h & 3 != 3) {
            let Some(w) = compressed(h as u16) else {
                continue;
            };
            found += 1;
            if let Some(h2) = compress(w) {
                spelt += 1;
                assert_eq!(compressed(h2), Some(w), "{h:#06x} and {h2:#06x}");
            } else {
                let d = decode(w);
                assert!(
                    d.rd == 0 || (d.kind == Kind::Addi && d.imm == 0),
                    "{h:#06x}: {} has no compressed spelling",
                    disasm(w)
                );
            }
        }
        assert!(spelt > found * 9 / 10, "{spelt} of {found}");
    }
}
