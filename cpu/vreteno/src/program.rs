// SPDX-License-Identifier: Apache-2.0
//! Programs for the core, as words: a small assembler with labels,
//! the demonstration program, and a generator of random straight-line
//! programs for the lockstep test.
use crate::isa::*;
use crate::model::DATA_BASE;

/// Words with labels: a forward branch is emitted with a placeholder
/// and patched when its label is placed.
#[derive(Default)]
pub struct Asm {
    pub words: Vec<u32>,
    fixups: Vec<(usize, usize, Box<dyn Fn(i32) -> u32>)>,
    abs_fixups: Vec<(usize, usize, Box<dyn Fn(u32) -> u32>)>,
    labels: Vec<Option<usize>>,
}

impl Asm {
    pub fn emit(&mut self, w: u32) {
        self.words.push(w);
    }
    /// A fresh label, not yet placed.
    pub fn label(&mut self) -> usize {
        self.labels.push(None);
        self.labels.len() - 1
    }
    /// Place a label here.
    pub fn place(&mut self, l: usize) {
        self.labels[l] = Some(self.words.len());
        let here = self.words.len();
        for (at, lbl, enc) in &self.fixups {
            if *lbl == l {
                self.words[*at] = enc((here as i32 - *at as i32) * 4);
            }
        }
        for (at, lbl, enc) in &self.abs_fixups {
            if *lbl == l {
                self.words[*at] = enc(here as u32 * 4);
            }
        }
    }
    /// A label's absolute address, `enc` taking it: what a handler's
    /// address in mtvec needs. The program is at zero.
    pub fn abs(&mut self, l: usize, enc: impl Fn(u32) -> u32 + 'static) {
        let at = self.words.len();
        match self.labels[l] {
            Some(t) => self.words.push(enc(t as u32 * 4)),
            None => {
                self.abs_fixups.push((at, l, Box::new(enc)));
                self.words.push(0);
            }
        }
    }
    /// A branch or jump to a label, `enc` taking the byte offset.
    pub fn to(&mut self, l: usize, enc: impl Fn(i32) -> u32 + 'static) {
        let at = self.words.len();
        match self.labels[l] {
            Some(t) => self.words.push(enc((t as i32 - at as i32) * 4)),
            None => {
                self.fixups.push((at, l, Box::new(enc)));
                self.words.push(0);
            }
        }
    }
}

/// The demonstration: a loop that sums one to ten, a call that
/// doubles the sum, stores and loads of every width with the sign
/// extension they imply, the upper immediates, a trap handler that
/// an ecall and an illegal word reach and return from, the CSRs,
/// a use of a word the instruction before it loaded, which stalls a
/// cycle, the multiplies and divides, then `ebreak`. It leaves 110 in
/// x10 and at the first data word, the second trap's cause in x23, 5
/// in x24, 0xfe01 in x25, -220 in x26, -55 in x29 and -2 in x30.
pub fn demo() -> Vec<u32> {
    let mut a = Asm::default();
    let (top, done, double) = (a.label(), a.label(), a.label());
    let handler = a.label();
    a.emit(lui(2, DATA_BASE >> 12)); // x2 = data base
    a.emit(addi(5, 0, 10)); // x5 = 10, the count
    a.emit(addi(10, 0, 0)); // x10 = 0, the sum
    a.emit(addi(6, 0, 0)); // x6 = 0, i
    a.place(top);
    a.emit(addi(6, 6, 1)); // i += 1
    a.emit(add(10, 10, 6)); // sum += i
    a.to(top, |o| bne(6, 5, o)); // until i == 10
    a.to(double, |o| jal(1, o)); // x10 = double(x10)
    a.emit(sw(10, 2, 0)); // mem[0] = 110
    a.emit(addi(7, 0, -2)); // x7 = -2
    a.emit(sb(7, 2, 5)); // a byte of it at 5
    a.emit(sh(7, 2, 10)); // a half of it at 10
    a.emit(lb(11, 2, 5)); // x11 = -2, sign extended
    a.emit(lbu(12, 2, 5)); // x12 = 254
    a.emit(lh(13, 2, 10)); // x13 = -2
    a.emit(lhu(14, 2, 10)); // x14 = 65534
    a.emit(lw(15, 2, 4)); // x15 = the byte in its word
    a.emit(addi(25, 15, 1)); // x25 = x15 + 1, a use right after the load
    a.emit(auipc(16, 1)); // x16 = pc + 4096
    a.emit(srai(17, 7, 1)); // x17 = -1
    a.emit(slt(18, 7, 0)); // x18 = 1: -2 < 0
    a.emit(sltu(19, 7, 0)); // x19 = 0: big unsigned
    a.emit(xori(20, 7, -1)); // x20 = 1

    // The M extension: each takes thirty-three cycles in the core.
    a.emit(mul(26, 10, 7)); // x26 = 110 * -2 = -220
    a.emit(mulh(27, 7, 7)); // x27 = high word of 4 = 0
    a.emit(mulhu(28, 7, 7)); // x28 = high word of 0xfffffffe^2
    a.emit(div(29, 10, 7)); // x29 = 110 / -2 = -55
    a.emit(rem(30, 7, 5)); // x30 = -2 rem 10 = -2

    // Traps: the handler's address into mtvec, an ecall, an illegal
    // word, each returning to the word after it; then the CSRs.
    a.abs(handler, |h| addi(21, 0, h as i32)); // x21 = the handler
    a.emit(csrrw(0, CSR_MTVEC, 21)); // mtvec = x21
    a.emit(ecall()); // trap, cause 11
    a.emit(0); // an illegal word: trap, cause 2
    a.emit(csrrwi(0, CSR_MSCRATCH, 5)); // mscratch = 5
    a.emit(csrrs(24, CSR_MSCRATCH, 0)); // x24 = mscratch
    a.to(done, |o| jal(0, o));
    a.place(double);
    a.emit(add(10, 10, 10));
    a.emit(jalr(0, 1, 0)); // return
    a.place(handler);
    a.emit(csrrs(22, CSR_MEPC, 0)); // x22 = mepc
    a.emit(addi(22, 22, 4)); // past the trapping word
    a.emit(csrrw(0, CSR_MEPC, 22)); // mepc = x22
    a.emit(csrrs(23, CSR_MCAUSE, 0)); // x23 = mcause
    a.emit(mret());
    a.place(done);
    a.emit(ebreak());
    a.words
}

/// A random straight-line program: register operations, the M
/// extension among them, aligned stores and loads within the first
/// data words, a forward branch now and then, CSR operations on
/// mscratch, an ecall or an illegal
/// word now and then, which a handler after the end returns from,
/// and `ebreak` at the end. `seed` is the whole of it.
pub fn random(seed: u64, len: usize) -> Vec<u32> {
    let mut s = seed.wrapping_mul(0x9e3779b97f4a7c15) | 1;
    let mut next = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let mut a = Asm::default();
    let handler = a.label();
    a.emit(lui(2, DATA_BASE >> 12));
    a.abs(handler, |h| addi(31, 0, h as i32));
    a.emit(csrrw(0, CSR_MTVEC, 31));
    while a.words.len() < len {
        let r = next();
        let rd = (r >> 8 & 31) as u32;
        let rs1 = (r >> 16 & 31) as u32;
        let rs2 = (r >> 24 & 31) as u32;
        let imm = ((r >> 32) as i32) >> 20;
        let amt = (r >> 40 & 31) as u32;
        let off = ((r >> 48) & 0x3f) as i32 * 4; // an aligned data offset
        let w = match r & 31 {
            0 => addi(rd, rs1, imm),
            1 => slti(rd, rs1, imm),
            2 => sltiu(rd, rs1, imm),
            3 => xori(rd, rs1, imm),
            4 => ori(rd, rs1, imm),
            5 => andi(rd, rs1, imm),
            6 => slli(rd, rs1, amt),
            7 => srli(rd, rs1, amt),
            8 => srai(rd, rs1, amt),
            9 => add(rd, rs1, rs2),
            10 => sub(rd, rs1, rs2),
            11 => sll(rd, rs1, rs2),
            12 => slt(rd, rs1, rs2),
            13 => sltu(rd, rs1, rs2),
            14 => xor(rd, rs1, rs2),
            15 => srl(rd, rs1, rs2),
            16 => sra(rd, rs1, rs2),
            // Half of the time an M instruction, by the function code.
            17 => match r >> 60 & 15 {
                0 => mul(rd, rs1, rs2),
                1 => mulh(rd, rs1, rs2),
                2 => mulhsu(rd, rs1, rs2),
                3 => mulhu(rd, rs1, rs2),
                4 => div(rd, rs1, rs2),
                5 => divu(rd, rs1, rs2),
                6 => rem(rd, rs1, rs2),
                7 => remu(rd, rs1, rs2),
                _ => or(rd, rs1, rs2),
            },
            18 => and(rd, rs1, rs2),
            19 => lui(rd, (r >> 12) as u32 & 0xfffff),
            20 => auipc(rd, (r >> 12) as u32 & 0xfffff),
            21 => sw(rs2, 2, off),
            22 => sh(rs2, 2, off + (amt as i32 & 2)),
            23 => sb(rs2, 2, off + (amt as i32 & 3)),
            24 => lw(rd, 2, off),
            25 => lh(rd, 2, off + (amt as i32 & 2)),
            26 => lb(rd, 2, off + (amt as i32 & 3)),
            27 => lhu(rd, 2, off + (amt as i32 & 2)),
            28 => lbu(rd, 2, off + (amt as i32 & 3)),
            // The CSR instructions, on mscratch.
            29 => match r >> 60 & 7 {
                0 => csrrw(rd, CSR_MSCRATCH, rs1),
                1 => csrrs(rd, CSR_MSCRATCH, rs1),
                2 => csrrc(rd, CSR_MSCRATCH, rs1),
                3 => csrrwi(rd, CSR_MSCRATCH, amt),
                4 => csrrsi(rd, CSR_MSCRATCH, amt),
                5 => csrrci(rd, CSR_MSCRATCH, amt),
                6 => csrrs(rd, CSR_MCAUSE, 0),
                _ => csrrs(rd, CSR_MEPC, 0),
            },
            // A trap: an ecall, or an illegal word.
            30 => {
                if r >> 60 & 1 == 0 {
                    ecall()
                } else {
                    0
                }
            }
            // A forward branch over the next one or two words.
            _ => {
                let l = a.label();
                let f3 = (r >> 5 & 7) as u32;
                let skip = 1 + (r >> 60 & 1) as usize;
                let enc: fn(u32, u32, i32) -> u32 = match f3 {
                    0 => beq,
                    1 => bne,
                    2 | 3 => blt,
                    4 => bge,
                    5 => bltu,
                    _ => bgeu,
                };
                a.to(l, move |o| enc(rs1, rs2, o));
                for _ in 0..skip {
                    let r = next();
                    let rd = (r >> 8 & 31) as u32;
                    let rd = if rd == 2 { 3 } else { rd };
                    a.emit(addi(rd, (r >> 16 & 31) as u32, 1));
                }
                a.place(l);
                continue;
            }
        };
        // x2 stays the data base, so the loads and stores stay in range,
        // and x31 is the handler's own.
        if (rd == 2 || rd == 31) && !matches!(r & 31, 21..=23 | 30) {
            continue;
        }
        a.emit(w);
    }
    a.emit(ebreak());
    // The handler: return to the word after the one that trapped.
    a.place(handler);
    a.emit(csrrs(31, CSR_MEPC, 0));
    a.emit(addi(31, 31, 4));
    a.emit(csrrw(0, CSR_MEPC, 31));
    a.emit(mret());
    a.words
}
