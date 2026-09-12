// SPDX-License-Identifier: Apache-2.0
//! The reference: RV32I as a program, one `step` per instruction,
//! written against the decoder and nothing else. The core is checked
//! against it, in lockstep, every cycle.
use crate::isa::{
    decode, Kind, CAUSE_ECALL, CAUSE_ILLEGAL, CSR_MCAUSE, CSR_MEPC,
    CSR_MSCRATCH, CSR_MSTATUS, CSR_MTVEC,
};

/// Where data memory begins and how much there is, in bytes. The
/// program lives at zero, in its own memory; the two do not overlap
/// and a load from the program is a trap.
pub const DATA_BASE: u32 = 0x1000;
pub const DATA_BYTES: u32 = 4096;

/// What stops the model: the program's own `ebreak`, or a fault. An
/// illegal instruction and `ecall` do not stop it: they trap.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Halt {
    Break,
    Fault(u32),
}

/// The control and status registers: the five the core has. mstatus
/// holds two bits, MIE and MPIE; mtvec is a direct-mode base.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Csr {
    pub mstatus: u32,
    pub mtvec: u32,
    pub mscratch: u32,
    pub mepc: u32,
    pub mcause: u32,
}

/// The architectural state, and only that.
#[derive(Clone, Debug)]
pub struct Model {
    pub pc: u32,
    pub x: [u32; 32],
    pub mem: Vec<u32>,
    pub csr: Csr,
    pub halted: Option<Halt>,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            pc: 0,
            x: [0; 32],
            mem: vec![0; DATA_BYTES as usize / 4],
            csr: Csr::default(),
            halted: None,
        }
    }
}

const MIE: u32 = 1 << 3;
const MPIE: u32 = 1 << 7;

impl Model {
    /// A word of data memory, by byte address; `None` outside it.
    fn word(&self, addr: u32) -> Option<u32> {
        let off = addr.wrapping_sub(DATA_BASE);
        (off < DATA_BYTES).then(|| self.mem[(off / 4) as usize])
    }

    fn set_word(&mut self, addr: u32, v: u32) -> bool {
        let off = addr.wrapping_sub(DATA_BASE);
        if off < DATA_BYTES {
            self.mem[(off / 4) as usize] = v;
        }
        off < DATA_BYTES
    }

    fn csr_read(&self, addr: u32) -> Option<u32> {
        Some(match addr {
            CSR_MSTATUS => self.csr.mstatus,
            CSR_MTVEC => self.csr.mtvec,
            CSR_MSCRATCH => self.csr.mscratch,
            CSR_MEPC => self.csr.mepc,
            CSR_MCAUSE => self.csr.mcause,
            _ => return None,
        })
    }

    fn csr_write(&mut self, addr: u32, v: u32) {
        match addr {
            CSR_MSTATUS => self.csr.mstatus = v & (MIE | MPIE),
            CSR_MTVEC => self.csr.mtvec = v & !3,
            CSR_MSCRATCH => self.csr.mscratch = v,
            CSR_MEPC => self.csr.mepc = v & !1,
            CSR_MCAUSE => self.csr.mcause = v,
            _ => {}
        }
    }

    /// A trap: the cause and the instruction's address are saved, the
    /// interrupt enable is saved and cleared, and the handler is next.
    fn trap(&mut self, cause: u32) {
        self.csr.mepc = self.pc;
        self.csr.mcause = cause;
        let mie = self.csr.mstatus & MIE != 0;
        self.csr.mstatus = if mie { MPIE } else { 0 };
        self.pc = self.csr.mtvec;
    }

    /// One instruction. Does nothing once halted.
    pub fn step(&mut self, imem: &[u32]) {
        if self.halted.is_some() {
            return;
        }
        let Some(&w) = imem.get((self.pc / 4) as usize) else {
            self.halted = Some(Halt::Fault(self.pc));
            return;
        };
        let d = decode(w);
        let a = self.x[d.rs1 as usize];
        let b = self.x[d.rs2 as usize];
        let imm = d.imm as u32;
        let sh = (b & 31) as u32;
        let shi = (imm & 31) as u32;
        let mut next = self.pc.wrapping_add(4);
        let mut rd: Option<u32> = None;
        use Kind::*;
        match d.kind {
            Lui => rd = Some(imm),
            Auipc => rd = Some(self.pc.wrapping_add(imm)),
            Jal => {
                rd = Some(next);
                next = self.pc.wrapping_add(imm);
            }
            Jalr => {
                rd = Some(next);
                next = a.wrapping_add(imm) & !1;
            }
            Beq | Bne | Blt | Bge | Bltu | Bgeu => {
                let taken = match d.kind {
                    Beq => a == b,
                    Bne => a != b,
                    Blt => (a as i32) < (b as i32),
                    Bge => (a as i32) >= (b as i32),
                    Bltu => a < b,
                    _ => a >= b,
                };
                if taken {
                    next = self.pc.wrapping_add(imm);
                }
            }
            Lb | Lh | Lw | Lbu | Lhu => {
                let addr = a.wrapping_add(imm);
                let Some(word) = self.word(addr) else {
                    self.halted = Some(Halt::Fault(addr));
                    return;
                };
                let byte = (word >> (8 * (addr & 3))) & 0xff;
                let half = (word >> (16 * (addr >> 1 & 1))) & 0xffff;
                rd = Some(match d.kind {
                    Lb => byte as u8 as i8 as i32 as u32,
                    Lh => half as u16 as i16 as i32 as u32,
                    Lw => word,
                    Lbu => byte,
                    _ => half,
                });
            }
            Sb | Sh | Sw => {
                let addr = a.wrapping_add(imm);
                let Some(word) = self.word(addr) else {
                    self.halted = Some(Halt::Fault(addr));
                    return;
                };
                let v = match d.kind {
                    Sb => {
                        let lane = 8 * (addr & 3);
                        (word & !(0xff << lane)) | ((b & 0xff) << lane)
                    }
                    Sh => {
                        let lane = 16 * (addr >> 1 & 1);
                        (word & !(0xffff << lane)) | ((b & 0xffff) << lane)
                    }
                    _ => b,
                };
                self.set_word(addr, v);
            }
            Addi => rd = Some(a.wrapping_add(imm)),
            Slti => rd = Some(((a as i32) < (imm as i32)) as u32),
            Sltiu => rd = Some((a < imm) as u32),
            Xori => rd = Some(a ^ imm),
            Ori => rd = Some(a | imm),
            Andi => rd = Some(a & imm),
            Slli => rd = Some(a << shi),
            Srli => rd = Some(a >> shi),
            Srai => rd = Some(((a as i32) >> shi) as u32),
            Add => rd = Some(a.wrapping_add(b)),
            Sub => rd = Some(a.wrapping_sub(b)),
            Sll => rd = Some(a << sh),
            Slt => rd = Some(((a as i32) < (b as i32)) as u32),
            Sltu => rd = Some((a < b) as u32),
            Xor => rd = Some(a ^ b),
            Srl => rd = Some(a >> sh),
            Sra => rd = Some(((a as i32) >> sh) as u32),
            Or => rd = Some(a | b),
            And => rd = Some(a & b),
            Fence => {}
            Ebreak => {
                self.halted = Some(Halt::Break);
                return;
            }
            Ecall => {
                self.trap(CAUSE_ECALL);
                return;
            }
            Illegal => {
                self.trap(CAUSE_ILLEGAL);
                return;
            }
            Mret => {
                let mpie = self.csr.mstatus & MPIE != 0;
                self.csr.mstatus = MPIE | if mpie { MIE } else { 0 };
                next = self.csr.mepc;
            }
            Csrrw | Csrrs | Csrrc | Csrrwi | Csrrsi | Csrrci => {
                let Some(old) = self.csr_read(imm) else {
                    self.trap(CAUSE_ILLEGAL);
                    return;
                };
                let src = match d.kind {
                    Csrrw | Csrrs | Csrrc => a,
                    _ => d.rs1,
                };
                let v = match d.kind {
                    Csrrw | Csrrwi => src,
                    Csrrs | Csrrsi => old | src,
                    _ => old & !src,
                };
                self.csr_write(imm, v);
                rd = Some(old);
            }
        }
        if let Some(v) = rd {
            if d.rd != 0 {
                self.x[d.rd as usize] = v;
            }
        }
        self.pc = next;
    }
}
