// SPDX-License-Identifier: Apache-2.0
//! RV32IM as bits: the encoders a program is written with, and the
//! decoder the reference model reads with. The core decodes on its
//! own, from the fields of the word, so the two decoders check each
//! other.

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

/// The causes the core raises: two exceptions, and the external
/// interrupt, whose cause has the top bit set.
pub const CAUSE_ILLEGAL: u32 = 2;
pub const CAUSE_ECALL: u32 = 11;
pub const CAUSE_MEXT: u32 = 0x8000_000b;
pub const CAUSE_MTIMER: u32 = 0x8000_0007;
/// The external and the timer interrupt's bits in `mie` and `mip`.
pub const MEXT: u32 = 1 << 11;
pub const MTIMER: u32 = 1 << 7;
/// The timer's four words: mtime low and high, mtimecmp low and high.
pub const TIMER_BASE: u32 = 0x2000;
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
        Fence | Ecall | Ebreak | Mret => m,
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
}
