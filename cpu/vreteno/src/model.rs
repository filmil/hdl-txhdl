// SPDX-License-Identifier: Apache-2.0
//! The reference: RV32IMC as a program, one `step` per instruction,
//! written against the decoder and nothing else. The core is checked
//! against it, in lockstep, every cycle.
use crate::core::IMEM_BYTES;
use crate::isa::{
    compressed, decode, is_compressed, Kind, CAUSE_BREAKPOINT, CAUSE_ECALL,
    CAUSE_ILLEGAL, CAUSE_LOAD_MISALIGNED, CAUSE_MEXT, CAUSE_MSOFT,
    CAUSE_MTIMER, CAUSE_STORE_MISALIGNED, CLINT_BASE, CLINT_MASK, CSR_DCSR,
    CSR_DPC, CSR_MARCHID, CSR_MCAUSE, CSR_MCYCLE, CSR_MCYCLEH, CSR_MEPC,
    CSR_MHALT, CSR_MHARTID, CSR_MIE, CSR_MIMPID, CSR_MINSTRET, CSR_MINSTRETH,
    CSR_MIP, CSR_MISA, CSR_MSCRATCH, CSR_MSTATUS, CSR_MTVAL, CSR_MTVEC,
    CSR_MVENDORID, MEXT, MISA, MSOFT, MTIMECMP_OFF, MTIMER, UART_BASE,
};

/// Where data memory begins and how much there is, in bytes. The
/// program lives at zero, in its own memory; the two do not overlap
/// and a load from the program is a trap.
pub const DATA_BASE: u32 = 0x1000;
pub const DATA_BYTES: u32 = 4096;

/// What stops the model: the program writing `mhalt`, or a fault. An
/// illegal instruction, `ecall` and `ebreak` do not stop it: they
/// trap.
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
    /// Instructions retired. The core counts the same thing in
    /// `minstret`, and this model counts it by stepping, so the two
    /// can be compared; the cycles beside it cannot, which
    /// `csr_read` says.
    pub minstret: u64,
    pub uart: Vec<u8>,
    /// The timer's line as the core saw it, set by the caller with the
    /// count; the timer is a device on the bus, so its pending bit is
    /// what the line says, not what the model could compute.
    pub tirq: bool,
    /// The software interrupt the program raised for itself, which is
    /// bit 0 of `msip` in the interrupt controller.
    pub msip: bool,
    pub halted: Option<Halt>,
    /// Debug mode (issue 154): halted by a debugger and resumable. The
    /// core enters it before an instruction, on a halt request, on the
    /// instruction after a single step, or on an `ebreak` that
    /// `dcsr.ebreakm` sends here; the harness tells the model when the
    /// core has, since the model steps only on a retirement and an
    /// entry retires nothing.
    pub debug: bool,
    /// The instruction to execute on resume.
    pub dpc: u32,
    /// `dcsr` as read: version 4, `ebreakm`, the cause, `step`, privilege 3.
    pub dcsr: u32,
    /// A single step: armed by a resume with `dcsr.step`, and once one
    /// instruction has run, stepped, which asks to enter again.
    pub step_armed: bool,
    pub stepped: bool,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            pc: 0,
            x: [0; 32],
            mem: vec![0; DATA_BYTES as usize / 4],
            csr: Csr::default(),
            dev_word: 0,
            // All ones, as the timer's reset leaves it: a compare of zero
            // beside a count of zero is an interrupt pending from the
            // first cycle (issue 419).
            mtimecmp: u64::MAX,
            minstret: 0,
            uart: Vec::new(),
            tirq: false,
            msip: false,
            halted: None,
            debug: false,
            dpc: 0,
            dcsr: 0x4000_0003,
            step_armed: false,
            stepped: false,
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

/// Whether an access of this kind at this address is misaligned: a
/// half wants an even address and a word a multiple of four, and a
/// byte is never misaligned. The core raises an exception rather than
/// supporting such an access, which the specification allows and issue
/// 138 asked for, so the model has to agree or the lockstep test will
/// say the core is wrong.
pub fn misaligned(kind: Kind, addr: u32) -> bool {
    use Kind::*;
    match kind {
        Lh | Lhu | Sh => addr & 1 != 0,
        Lw | Sw => addr & 3 != 0,
        _ => false,
    }
}

/// The first address above the data memory: everything there and
/// beyond is a device's, and what a load there answers is what the bus
/// gave the core, which the caller hands over.
const DEVICES: u32 = DATA_BASE + DATA_BYTES;

impl Model {
    /// The instruction at `pc`, from the boot memory below
    /// `IMEM_BYTES` or from the data memory above it, which is what
    /// the core does once it fetches from the bus (issue 134).
    fn fetch_at(&self, imem: &[u32], pc: u32) -> Option<(u32, u32)> {
        if pc < IMEM_BYTES {
            return fetch(imem, pc);
        }
        let half = |at: u32| -> Option<u16> {
            let off = at.wrapping_sub(DATA_BASE);
            (off < DATA_BYTES).then(|| {
                (self.mem[(off / 4) as usize] >> (8 * (at & 2))) as u16
            })
        };
        let lo = half(pc)?;
        if crate::isa::is_compressed(lo) {
            return Some((crate::isa::compressed(lo).unwrap_or(lo as u32), 2));
        }
        let hi = half(pc.wrapping_add(2))?;
        Some(((hi as u32) << 16 | lo as u32, 4))
    }

    /// A word by byte address: of the boot memory below `IMEM_BYTES`,
    /// which is on the bus read-only and holds a program's constants
    /// (issue 268); of the data memory; or, above it, the word the bus
    /// answered the core with, which the caller handed over. `None`
    /// between the boot memory and the data memory.
    fn word(&self, imem: &[u32], addr: u32) -> Option<u32> {
        if addr < IMEM_BYTES {
            return Some(*imem.get((addr / 4) as usize).unwrap_or(&0));
        }
        if addr >= DEVICES {
            return Some(self.dev_word);
        }
        let off = addr.wrapping_sub(DATA_BASE);
        (off < DATA_BYTES).then(|| self.mem[(off / 4) as usize])
    }

    /// A store's word. Above the data memory the store is a device's
    /// business; the model keeps what it can check at the end, the
    /// timer's compare and the bytes given to the serial port. A store
    /// into the boot memory is refused by the memory and ignored by the
    /// core, which does not look at a write's answer, so it changes
    /// nothing here either.
    fn set_word(&mut self, addr: u32, v: u32) -> bool {
        if addr < IMEM_BYTES {
            return true;
        }
        if addr & CLINT_MASK == CLINT_BASE {
            let off = addr & !CLINT_MASK;
            if off == MTIMECMP_OFF || off == MTIMECMP_OFF + 4 {
                let shift = 8 * (off & 4);
                self.mtimecmp = (self.mtimecmp & !(0xffff_ffff << shift))
                    | (v as u64) << shift;
            }
            if off == 0 {
                self.msip = v & 1 == 1;
            }
            return true;
        }
        if addr == UART_BASE {
            self.uart.push(v as u8);
            return true;
        }
        if addr >= DEVICES {
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

    /// The software interrupt's pending bit: the controller's `msip`,
    /// as the core sees it on its line.
    fn msip(&self) -> u32 {
        if self.msip {
            MSOFT
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
            CSR_MIP => self.csr.mip | self.mtip() | self.msip(),
            CSR_MTVAL => self.csr.mtval,
            // The halt holds nothing: it reads as zero, and a write of
            // an odd value to it stops the machine.
            CSR_MHALT => 0,
            CSR_DCSR => self.dcsr,
            CSR_DPC => self.dpc,
            // What the machine is. `misa` says RV32IMC; the four
            // machine information registers say that the vendor, the
            // architecture and the implementation are unassigned and
            // that this is hart zero.
            CSR_MISA => MISA,
            CSR_MVENDORID | CSR_MARCHID | CSR_MIMPID | CSR_MHARTID => 0,
            // The counters are legal here, so that a program reading
            // one traps in neither the core nor the model. What they
            // read is another matter: this model steps on a
            // retirement and has no idea how many cycles the pipeline
            // spent, so it counts what it can, which is retirements,
            // and answers zero for the cycles. A lockstep program must
            // therefore not read `mcycle` into a register, and the
            // counters are checked by a directed run instead, without
            // a model beside it.
            CSR_MINSTRET => self.minstret as u32,
            CSR_MINSTRETH => (self.minstret >> 32) as u32,
            CSR_MCYCLE | CSR_MCYCLEH => 0,
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
            CSR_MIE => self.csr.mie = v & (MEXT | MSOFT | MTIMER),
            CSR_MIP => self.csr.mip = v & MEXT,
            CSR_MTVAL => self.csr.mtval = v,
            // `ebreakm` and `step` are the program's; the rest is the
            // core's to say.
            CSR_DCSR => {
                self.dcsr = 0x4000_0003 | (v & 0x8004) | (self.dcsr & 0x1c0)
            }
            CSR_DPC => self.dpc = v & !1,
            CSR_MINSTRET => {
                self.minstret = (self.minstret & !0xffff_ffff) | u64::from(v)
            }
            CSR_MINSTRETH => {
                self.minstret =
                    (self.minstret & 0xffff_ffff) | (u64::from(v) << 32)
            }
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
        let pending = (self.csr.mip | self.mtip() | self.msip()) & self.csr.mie;
        // The order the specification gives: the external interrupt
        // first, then the software one, then the timer's.
        if pending & MEXT != 0 {
            Some(CAUSE_MEXT)
        } else if pending & MSOFT != 0 {
            Some(CAUSE_MSOFT)
        } else if pending & MTIMER != 0 {
            Some(CAUSE_MTIMER)
        } else {
            None
        }
    }

    /// One instruction, or the interrupt taken instead of it when
    /// `interrupt` names one: the caller decides, since the core decides
    /// on the pending bits and the count as they stood a cycle earlier.
    /// Debug mode entered, before the instruction at `pc`, which
    /// becomes `dpc`: the cause is a step when one has just run, an
    /// `ebreak` when that is the instruction and `dcsr.ebreakm` is set,
    /// and a halt request otherwise.
    pub fn enter_debug(&mut self, imem: &[u32]) {
        let at_ebreak = self
            .fetch_at(imem, self.pc)
            .map(|(w, _)| matches!(decode(w).kind, Kind::Ebreak))
            .unwrap_or(false);
        let cause = if self.stepped {
            4
        } else if at_ebreak && self.dcsr & 0x8000 != 0 {
            1
        } else {
            3
        };
        self.dcsr = 0x4000_0003 | (self.dcsr & 0x8004) | (cause << 6);
        self.dpc = self.pc;
        self.debug = true;
        self.stepped = false;
    }

    /// The reset line: the program counter to zero, every CSR and both
    /// counters as configuration left them, the timer's compare to all
    /// ones, debug mode left and the halt ended. The register file and
    /// the memory keep what they hold, as the hardware's do; so do the
    /// bytes the serial port was given, which are the run's record
    /// rather than the machine's state (issue 419).
    pub fn reset(&mut self) {
        self.pc = 0;
        self.csr = Csr::default();
        self.minstret = 0;
        self.mtimecmp = u64::MAX;
        self.halted = None;
        self.debug = false;
        self.dpc = 0;
        self.dcsr = 0x4000_0003;
        self.step_armed = false;
        self.stepped = false;
    }

    /// Debug mode left, to `dpc`, arming a single step when `dcsr.step`
    /// asks for one.
    pub fn resume(&mut self) {
        self.debug = false;
        self.pc = self.dpc;
        self.step_armed = self.dcsr & 4 != 0;
    }

    /// Does nothing once halted.
    pub fn step(&mut self, imem: &[u32], interrupt: Option<u32>) {
        if self.halted.is_some() || self.debug {
            return;
        }
        // One more retired, counted before the instruction runs so
        // that a read of `minstret` by this instruction does not count
        // itself, which is what the specification asks for.
        self.minstret = self.minstret.wrapping_add(1);
        let Some((w, len)) = self.fetch_at(imem, self.pc) else {
            self.halted = Some(Halt::Fault(self.pc));
            return;
        };
        if let Some(cause) = interrupt {
            self.trap(cause, 0);
            return;
        }
        // The instruction about to run is the stepped one, if a step
        // was armed: the core asks to enter again before the next.
        if self.step_armed {
            self.step_armed = false;
            self.stepped = true;
        }
        let d = decode(w);
        let a = self.x[d.rs1 as usize];
        let b = self.x[d.rs2 as usize];
        let imm = d.imm as u32;
        let sh = (b & 31) as u32;
        let shi = (imm & 31) as u32;
        let mut next = self.pc.wrapping_add(len);
        let mut rd: Option<u32> = None;
        // Whether this instruction is the one that stops the
        // machine: a write of an odd value to `mhalt`.
        let mut halting = false;
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
                // A half wants an even address and a word a multiple of
                // four. The core raises the exception rather than
                // supporting the access, which the specification allows
                // and issue 138 asked for, so the model does too.
                if misaligned(d.kind, addr) {
                    self.trap(CAUSE_LOAD_MISALIGNED, addr);
                    return;
                }
                let Some(word) = self.word(imem, addr) else {
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
                if misaligned(d.kind, addr) {
                    self.trap(CAUSE_STORE_MISALIGNED, addr);
                    return;
                }
                let Some(word) = self.word(imem, addr) else {
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
                // A breakpoint, not a halt: a monitor catches it,
                // prints, steps, continues. A program that means to
                // stop writes `mhalt` instead, which is issue 139.
                self.trap(CAUSE_BREAKPOINT, self.pc);
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
            // The core waits for an interrupt rather than spinning,
            // but nothing retires while it waits, and the lockstep
            // steps this model on a retirement. So the model has no
            // waiting state: it retires `wfi` and moves on, and the
            // core's wait shows up as cycles in which it retires
            // nothing, which is what the test measures.
            Wfi => {}
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
                // A write to a register whose address begins with two
                // set bits is an illegal instruction. `csrrw` and
                // `csrrwi` always write; a set or a clear writes only
                // when its source field is not zero, which is what the
                // specification says and is not the same as the value
                // in the register being zero.
                let writes = matches!(d.kind, Csrrw | Csrrwi) || d.rs1 != 0;
                if writes
                    && matches!(
                        imm,
                        CSR_MVENDORID | CSR_MARCHID | CSR_MIMPID | CSR_MHARTID
                    )
                {
                    self.trap(CAUSE_ILLEGAL, w);
                    return;
                }
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
                // A write of an odd value to `mhalt` is how a program
                // says it is finished. The register holds nothing and
                // reads as zero, and the instruction retires as any
                // other does before the machine stops, which is what
                // the core does too.
                halting = imm == CSR_MHALT && v & 1 != 0;
            }
        }
        if let Some(v) = rd {
            if d.rd != 0 {
                self.x[d.rd as usize] = v;
            }
        }
        self.pc = next;
        if halting {
            self.halted = Some(Halt::Break);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isa::{addi, c_addi, c_jal, c_jr, c_nop, halt, lui};

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
            halt(),                                      // 12
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

    /// An unaligned load and an unaligned store each trap, with the
    /// cause the specification names and the address as the trap
    /// value. A byte never traps, whatever the address, and the
    /// aligned forms still work. This is issue 138: each of these used
    /// to use the aligned word and say nothing.
    #[test]
    fn an_unaligned_access_traps() {
        use crate::isa::{csrrw, lb, lh, lw, sh, sw};
        let handler = 0x40;
        // The trap handler is a word past everything else, and every
        // program below sets it, does one access, and ends.
        let run = |access: u32| {
            let mut imem = vec![
                lui(1, 0x1),            // 0: x1 = 0x1000, the data
                addi(2, 0, 1),          // 4: x2 = 1, a byte to store
                addi(3, 0, handler),    // 8: x3 = the handler
                csrrw(0, CSR_MTVEC, 3), // 12: mtvec = x3
                access,                 // 16
                halt(),                 // 20
            ];
            imem.resize(32, halt());
            let mut m = Model::default();
            for _ in 0..8 {
                if m.halted.is_some() {
                    break;
                }
                m.step(&imem, None);
            }
            m
        };
        // The aligned forms go through and leave no cause behind.
        let m = run(lw(4, 1, 0));
        assert_eq!(m.csr.mcause, 0, "an aligned word is no trap");
        let m = run(lb(4, 1, 3));
        assert_eq!(m.csr.mcause, 0, "a byte at any address is no trap");
        // The unaligned ones trap, with the address they asked for.
        let m = run(lw(4, 1, 2));
        assert_eq!(m.csr.mcause, CAUSE_LOAD_MISALIGNED, "lw at 0x1002");
        assert_eq!(m.csr.mtval, 0x1002, "the address is the trap value");
        assert_eq!(m.csr.mepc, 16, "the access that trapped");
        // The handler's first word is one of the halts the fill above
        // put there, and a halt retires before the machine stops, so
        // the program counter stands one word past the entry.
        assert_eq!(m.pc, handler as u32 + 4, "and the handler ran");
        let m = run(lh(4, 1, 1));
        assert_eq!(m.csr.mcause, CAUSE_LOAD_MISALIGNED, "lh at 0x1001");
        let m = run(sw(2, 1, 1));
        assert_eq!(m.csr.mcause, CAUSE_STORE_MISALIGNED, "sw at 0x1001");
        assert_eq!(m.csr.mtval, 0x1001);
        let m = run(sh(2, 1, 3));
        assert_eq!(m.csr.mcause, CAUSE_STORE_MISALIGNED, "sh at 0x1003");
        // And the store that trapped wrote nothing.
        assert_eq!(m.mem[0], 0, "a trapping store leaves the word alone");
    }
}
