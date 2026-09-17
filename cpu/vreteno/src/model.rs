// SPDX-License-Identifier: Apache-2.0
//! The reference: RV32IMC as a program, one `step` per instruction,
//! written against the decoder and nothing else. The core is checked
//! against it, in lockstep, every cycle.
use crate::isa::{
    compressed, decode, is_compressed, Kind, CAUSE_ECALL, CAUSE_ILLEGAL,
    CAUSE_MEXT, CAUSE_MTIMER, CSR_MCAUSE, CSR_MEPC, CSR_MIE, CSR_MIP,
    CSR_MSCRATCH, CSR_MSTATUS, CSR_MTVAL, CSR_MTVEC, MEXT, MTIMER, TIMER_BASE,
    UART_BASE,
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
    /// What the bus answered the core's last load from a device: the
    /// model has neither a clock nor a bus, so the caller sets it
    /// before the step of a device load. The timer's compare and the
    /// bytes the serial port was given are the model's own.
    pub dev_word: u32,
    pub mtimecmp: u64,
    pub uart: Vec<u8>,
    /// The timer's line as the core saw it, set by the caller with the
    /// count; the timer is a device on the bus, so its pending bit is
    /// what the line says, not what the model could compute.
    pub tirq: bool,
    pub halted: Option<Halt>,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            pc: 0,
            x: [0; 32],
            mem: vec![0; DATA_BYTES as usize / 4],
            csr: Csr::default(),
            dev_word: 0,
            mtimecmp: 0,
            uart: Vec::new(),
            tirq: false,
            halted: None,
        }
    }
}

const MIE: u32 = 1 << 3;
const MPIE: u32 = 1 << 7;

/// The instruction at `pc` in the instruction memory, as thirty-two
/// bits, and its length in bytes; `None` past the memory's end. The
/// words are little-endian, so a halfword at an address with bit 1 set
/// is the upper half of its word, and a thirty-two bit instruction
/// there takes its upper half from the next word. A compressed
/// instruction reads as the one it stands for, and a halfword that is
/// none as itself, which decodes as illegal and is the trap value an
/// illegal instruction leaves.
pub fn fetch(imem: &[u32], pc: u32) -> Option<(u32, u32)> {
    let half = |at: u32| {
        imem.get((at / 4) as usize)
            .map(|w| (w >> (8 * (at & 2))) as u16)
    };
    let lo = half(pc)?;
    if is_compressed(lo) {
        return Some((compressed(lo).unwrap_or(lo as u32), 2));
    }
    let hi = half(pc.wrapping_add(2))?;
    Some(((hi as u32) << 16 | lo as u32, 4))
}

impl Model {
    /// A word of data memory by byte address, or, above it, the word
    /// the bus answered the core with, which the caller handed over;
    /// `None` below the data memory.
    fn word(&self, addr: u32) -> Option<u32> {
        if addr >= TIMER_BASE {
            return Some(self.dev_word);
        }
        let off = addr.wrapping_sub(DATA_BASE);
        (off < DATA_BYTES).then(|| self.mem[(off / 4) as usize])
    }

    /// A store's word. Above the data memory the store is a device's
    /// business; the model keeps what it can check at the end, the
    /// timer's compare and the bytes given to the serial port.
    fn set_word(&mut self, addr: u32, v: u32) -> bool {
        let t = addr.wrapping_sub(TIMER_BASE);
        if t < 16 {
            if t >= 8 {
                let shift = 8 * (t & 4);
                self.mtimecmp = (self.mtimecmp & !(0xffff_ffff << shift))
                    | (v as u64) << shift;
            }
            return true;
        }
        if addr == UART_BASE {
            self.uart.push(v as u8);
            return true;
        }
        if addr >= TIMER_BASE {
            return true;
        }
        let off = addr.wrapping_sub(DATA_BASE);
        if off < DATA_BYTES {
            self.mem[(off / 4) as usize] = v;
        }
        off < DATA_BYTES
    }

    /// The timer's pending bit: its line, as the core saw it.
    fn mtip(&self) -> u32 {
        if self.tirq {
            MTIMER
        } else {
            0
        }
    }

    fn csr_read(&self, addr: u32) -> Option<u32> {
        Some(match addr {
            CSR_MSTATUS => self.csr.mstatus,
            CSR_MTVEC => self.csr.mtvec,
            CSR_MSCRATCH => self.csr.mscratch,
            CSR_MEPC => self.csr.mepc,
            CSR_MCAUSE => self.csr.mcause,
            CSR_MIE => self.csr.mie,
            CSR_MIP => self.csr.mip | self.mtip(),
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
            CSR_MIE => self.csr.mie = v & (MEXT | MTIMER),
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

    /// The interrupt that would be taken before the next instruction,
    /// if any: the external one first, then the timer's, each pending
    /// and enabled in `mie`, with interrupts enabled in `mstatus`.
    pub fn interrupt(&self) -> Option<u32> {
        if self.csr.mstatus & MIE == 0 {
            return None;
        }
        let pending = (self.csr.mip | self.mtip()) & self.csr.mie;
        if pending & MEXT != 0 {
            Some(CAUSE_MEXT)
        } else if pending & MTIMER != 0 {
            Some(CAUSE_MTIMER)
        } else {
            None
        }
    }

    /// One instruction, or the interrupt taken instead of it when
    /// `interrupt` names one: the caller decides, since the core decides
    /// on the pending bits and the count as they stood a cycle earlier.
    /// Does nothing once halted.
    pub fn step(&mut self, imem: &[u32], interrupt: Option<u32>) {
        if self.halted.is_some() {
            return;
        }
        let Some((w, len)) = fetch(imem, self.pc) else {
            self.halted = Some(Halt::Fault(self.pc));
            return;
        };
        if let Some(cause) = interrupt {
            self.trap(cause, 0);
            return;
        }
        let d = decode(w);
        let a = self.x[d.rs1 as usize];
        let b = self.x[d.rs2 as usize];
        let imm = d.imm as u32;
        let sh = (b & 31) as u32;
        let shi = (imm & 31) as u32;
        let mut next = self.pc.wrapping_add(len);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isa::{addi, c_addi, c_jal, c_jr, c_nop, ebreak, lui};

    /// A program of both widths: a compressed instruction, a thirty-two
    /// bit one that starts in the upper half of the first word and ends
    /// in the second, a compressed call whose link is two bytes on, and
    /// a compressed return to that link.
    #[test]
    fn half_words() {
        let lui5 = lui(5, 0x12345);
        let imem = [
            (lui5 & 0xffff) << 16 | c_addi(1, 7) as u32, // 0, 2
            (c_jal(10) as u32) << 16 | lui5 >> 16,       // 6: to 16
            addi(6, 0, 1),                               // 8
            ebreak(),                                    // 12
            (c_nop() as u32) << 16 | c_jr(1) as u32,     // 16: to 8
        ];
        assert_eq!(fetch(&imem, 0), Some((addi(1, 1, 7), 2)));
        assert_eq!(fetch(&imem, 2), Some((lui5, 4)), "across two words");
        assert_eq!(fetch(&imem, 20), None, "past the end");
        let mut m = Model::default();
        let mut pcs = vec![];
        while m.halted.is_none() {
            pcs.push(m.pc);
            m.step(&imem, None);
        }
        assert_eq!(pcs, [0, 2, 6, 16, 8, 12]);
        assert_eq!(m.halted, Some(Halt::Break));
        assert_eq!(m.x[1], 8, "the link is the address after c.jal");
        assert_eq!(m.x[5], 0x1234_5000);
        assert_eq!(m.x[6], 1);
    }
}
