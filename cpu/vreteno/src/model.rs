// SPDX-License-Identifier: Apache-2.0
//! The reference: RV32IM as a program, one `step` per instruction,
//! written against the decoder and nothing else. The core is checked
//! against it, in lockstep, every cycle.
use crate::isa::{
    decode, Kind, CAUSE_ECALL, CAUSE_ILLEGAL, CAUSE_MEXT, CSR_MCAUSE, CSR_MEPC,
    CSR_MIE, CSR_MIP, CSR_MSCRATCH, CSR_MSTATUS, CSR_MTVAL, CSR_MTVEC, MEXT,
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
    pub mie: u32,
    pub mip: u32,
    pub mtval: u32,
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
            CSR_MIE => self.csr.mie,
            CSR_MIP => self.csr.mip,
            CSR_MTVAL => self.csr.mtval,
            _ => return None,
        })
    }

    /// The external interrupt line, seen high: pending until software
    /// clears it in `mip`.
    pub fn raise(&mut self) {
        self.csr.mip |= MEXT;
    }

    fn csr_write(&mut self, addr: u32, v: u32) {
        match addr {
            CSR_MSTATUS => self.csr.mstatus = v & (MIE | MPIE),
            CSR_MTVEC => self.csr.mtvec = v & !3,
            CSR_MSCRATCH => self.csr.mscratch = v,
            CSR_MEPC => self.csr.mepc = v & !1,
            CSR_MCAUSE => self.csr.mcause = v,
            CSR_MIE => self.csr.mie = v & MEXT,
            CSR_MIP => self.csr.mip = v & MEXT,
            CSR_MTVAL => self.csr.mtval = v,
            _ => {}
        }
    }

    /// A trap: the cause, the instruction's address and the trap value
    /// are saved, the interrupt enable is saved and cleared, and the
    /// handler is next. The trap value is the word for an illegal
    /// instruction and zero otherwise.
    fn trap(&mut self, cause: u32, tval: u32) {
        self.csr.mepc = self.pc;
        self.csr.mcause = cause;
        self.csr.mtval = tval;
        let mie = self.csr.mstatus & MIE != 0;
        self.csr.mstatus = if mie { MPIE } else { 0 };
        self.pc = self.csr.mtvec;
    }

    /// Whether an interrupt would be taken before the next instruction:
    /// pending, enabled in `mie`, and interrupts enabled in `mstatus`.
    pub fn interrupt(&self) -> bool {
        self.csr.mip & self.csr.mie & MEXT != 0 && self.csr.mstatus & MIE != 0
    }

    /// One instruction, or the interrupt taken instead of it when
    /// `interrupt` says so: the caller decides, since the core decides
    /// on the pending bit as it stood a cycle earlier. Does nothing once
    /// halted.
    pub fn step(&mut self, imem: &[u32], interrupt: bool) {
        if self.halted.is_some() {
            return;
        }
        let Some(&w) = imem.get((self.pc / 4) as usize) else {
            self.halted = Some(Halt::Fault(self.pc));
            return;
        };
        if interrupt {
            self.trap(CAUSE_MEXT, 0);
            return;
        }
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
            // The M extension. A product's low word is the same for
            // every signedness; the high word depends on it. Division
            // by zero and the one overflow are what the manual says,
            // which is what Rust's wrapping division says too, except
            // for the zero, which Rust would refuse.
            Mul => rd = Some(a.wrapping_mul(b)),
            Mulh => {
                rd = Some(((a as i32 as i64 * b as i32 as i64) >> 32) as u32)
            }
            Mulhsu => rd = Some(((a as i32 as i64 * b as i64) >> 32) as u32),
            Mulhu => rd = Some(((a as u64 * b as u64) >> 32) as u32),
            Div => {
                rd = Some(if b == 0 {
                    u32::MAX
                } else {
                    (a as i32).wrapping_div(b as i32) as u32
                })
            }
            Divu => rd = Some(if b == 0 { u32::MAX } else { a / b }),
            Rem => {
                rd = Some(if b == 0 {
                    a
                } else {
                    (a as i32).wrapping_rem(b as i32) as u32
                })
            }
            Remu => rd = Some(if b == 0 { a } else { a % b }),
            Fence => {}
            Ebreak => {
                self.halted = Some(Halt::Break);
                return;
            }
            Ecall => {
                self.trap(CAUSE_ECALL, 0);
                return;
            }
            Illegal => {
                self.trap(CAUSE_ILLEGAL, w);
                return;
            }
            Mret => {
                let mpie = self.csr.mstatus & MPIE != 0;
                self.csr.mstatus = MPIE | if mpie { MIE } else { 0 };
                next = self.csr.mepc;
            }
            Csrrw | Csrrs | Csrrc | Csrrwi | Csrrsi | Csrrci => {
                let Some(old) = self.csr_read(imm) else {
                    self.trap(CAUSE_ILLEGAL, w);
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
