// SPDX-License-Identifier: Apache-2.0
//! Vreteno: a two-stage RV32I core. One process, and on every edge
//! two things at once: the fetch stage reads the instruction memory
//! at the program counter into the instruction register, and the
//! execute stage decodes the fields of the word in that register,
//! executes, writes the register file and the data memory, and, on a
//! taken branch or a jump, redirects the fetch and squashes the word
//! it fetched this cycle, which is the one-cycle penalty. Simulation
//! form: the decode and the selection are Rust `match`es over the
//! fields, which the lowering does not reach yet; the state, the
//! waits and the drives are the language's, so the lockstep test and
//! the waveform are real, and lowering is a later change to this
//! file and not a rewrite.
use txhdl::comp::{Clock, DefaultClock, In, Mem, Out, Reg, Unit};
use txhdl::funcs::{band, bor, bxor, eq, lt, lt_signed, shl, shr, sra};
use txhdl::types::{Bit, U};
use txhdl::{Trace, Value};

/// Words of instruction memory and of data memory. Data memory is at
/// `DATA_BASE`, as the model has it.
pub const IMEM_WORDS: usize = 1024;
pub const DMEM_WORDS: usize = 1024;
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

/// The core's outputs: halted, the instruction executed, the
/// writeback.
pub type Outputs = (Out<Bit>, Out<U<32>>, Out<Writeback>);

/// The state: the fetch stage's program counter; the instruction
/// register, its program counter and whether it holds an
/// instruction, which is the boundary between the stages; the halt;
/// and the three memories.
#[derive(Trace, Default)]
pub struct Vreteno {
    pub pc: Reg<U<32>>,
    pub ir: Reg<U<32>>,
    pub ir_pc: Reg<U<32>>,
    pub valid: Reg<Bit>,
    pub halted: Reg<Bit>,
    pub regs: Mem<U<32>, 32>,
    pub imem: Mem<U<32>, IMEM_WORDS>,
    pub dmem: Mem<U<32>, DMEM_WORDS>,
}

/// The arithmetic and logic unit: `f3` selects, `alt` is bit 30 of
/// the word where it matters, subtraction and the arithmetic shift.
fn alu(f3: U<3>, alt: Bit, a: U<32>, b: U<32>) -> U<32> {
    let k = b.raw() as usize & 31;
    match f3.raw() {
        0 if alt.to_bool() => a.wrapping_sub(b),
        0 => a.wrapping_add(b),
        1 => shl(a, k),
        2 => lt_signed(a, b).zext(),
        3 => lt(a, b).zext(),
        4 => bxor(a, b),
        5 if alt.to_bool() => sra(a, k),
        5 => shr(a, k),
        6 => bor(a, b),
        _ => band(a, b),
    }
}

/// The branch condition, by `f3`.
fn taken(f3: U<3>, a: U<32>, b: U<32>) -> Bit {
    match f3.raw() {
        0 => eq(a, b),
        1 => eq(a, b).not(),
        4 => lt_signed(a, b),
        5 => lt_signed(a, b).not(),
        6 => lt(a, b),
        _ => lt(a, b).not(),
    }
}

impl Vreteno {
    /// A core with its program loaded.
    pub fn with(program: &[u32]) -> Self {
        let words: Vec<U<32>> = program.iter().map(|&w| U::from(w)).collect();
        Vreteno {
            imem: Mem::with(&words),
            ..Default::default()
        }
    }

    fn data_word(&self, addr: U<32>) -> usize {
        (addr.raw() as u32).wrapping_sub(DATA_BASE) as usize / 4
    }

    /// The architectural program counter: that of the instruction
    /// about to execute, or the fetch's when the stage is empty. What
    /// the model's program counter is compared against.
    pub fn arch_pc(&self) -> U<32> {
        if self.valid.get().to_bool() {
            self.ir_pc.get()
        } else {
            self.pc.get()
        }
    }
}

impl Unit<In<Bit>, Outputs> for Vreteno {
    async fn run(&mut self, rst: In<Bit>, (halt, instr, wb): Outputs) {
        loop {
            DefaultClock::rising().await;
            let rst = rst.get();
            let (fetch_pc, halted) = (self.pc.get(), self.halted.get());
            let (ir, pc, valid) =
                (self.ir.get(), self.ir_pc.get(), self.valid.get());
            // The fetch stage: the word at the program counter, into
            // the instruction register unless the execute stage
            // redirects below.
            let fetched = self.imem.read(fetch_pc.raw() as usize / 4);
            // The execute stage: the fields of the word.
            let opcode = ir.slice::<0, 7>();
            let rd = ir.slice::<7, 5>();
            let f3 = ir.slice::<12, 3>();
            let rs1 = ir.slice::<15, 5>();
            let rs2 = ir.slice::<20, 5>();
            let alt = ir.bit(30);
            // The five immediates, each a sign-extended word.
            let imm_i = ir.slice::<20, 12>().sext::<32>();
            let imm_s = ir
                .slice::<25, 7>()
                .concat::<5, 12>(ir.slice::<7, 5>())
                .sext::<32>();
            let imm_b = ir
                .slice::<31, 1>()
                .concat::<1, 2>(ir.slice::<7, 1>())
                .concat::<6, 8>(ir.slice::<25, 6>())
                .concat::<4, 12>(ir.slice::<8, 4>())
                .concat::<1, 13>(U::<1>::from(0u8))
                .sext::<32>();
            let imm_u =
                ir.slice::<12, 20>().concat::<12, 32>(U::<12>::from(0u8));
            let imm_j = ir
                .slice::<31, 1>()
                .concat::<8, 9>(ir.slice::<12, 8>())
                .concat::<1, 10>(ir.slice::<20, 1>())
                .concat::<10, 20>(ir.slice::<21, 10>())
                .concat::<1, 21>(U::<1>::from(0u8))
                .sext::<32>();
            let a = self.regs.read(rs1.raw() as usize);
            let b = self.regs.read(rs2.raw() as usize);
            let pc4 = pc.wrapping_add(U::from(4u8));

            let mut next = pc4;
            let mut write: Option<U<32>> = None;
            let mut stop = Bit::Zero;
            let run = !rst.to_bool() && !halted.to_bool() && valid.to_bool();
            match opcode.raw() as u32 {
                _ if !run => {
                    stop = halted;
                }
                0x37 => write = Some(imm_u),
                0x17 => write = Some(pc.wrapping_add(imm_u)),
                0x6f => {
                    write = Some(pc4);
                    next = pc.wrapping_add(imm_j);
                }
                0x67 => {
                    write = Some(pc4);
                    next = a.wrapping_add(imm_i).and(U::from(1u8).not());
                }
                0x63 => {
                    if taken(f3, a, b).to_bool() {
                        next = pc.wrapping_add(imm_b);
                    }
                }
                0x03 => {
                    let addr = a.wrapping_add(imm_i);
                    let word = self.dmem.read(self.data_word(addr));
                    let lane = addr.raw() as usize & 3;
                    let byte = shr(word, 8 * lane).slice::<0, 8>();
                    let half = shr(word, 16 * (lane >> 1)).slice::<0, 16>();
                    write = Some(match f3.raw() {
                        0 => byte.sext::<32>(),
                        1 => half.sext::<32>(),
                        2 => word,
                        4 => byte.zext::<32>(),
                        _ => half.zext::<32>(),
                    });
                }
                0x23 => {
                    let addr = a.wrapping_add(imm_s);
                    let at = self.data_word(addr);
                    let word = self.dmem.read(at);
                    let lane = addr.raw() as usize & 3;
                    let v = match f3.raw() {
                        0 => {
                            let mask = shl(U::from(0xffu32), 8 * lane);
                            bor(
                                band(word, mask.not()),
                                band(shl(b, 8 * lane), mask),
                            )
                        }
                        1 => {
                            let mask =
                                shl(U::from(0xffffu32), 16 * (lane >> 1));
                            bor(
                                band(word, mask.not()),
                                band(shl(b, 16 * (lane >> 1)), mask),
                            )
                        }
                        _ => b,
                    };
                    self.dmem.write(at, v);
                }
                // Register-immediate: `alt` only means something to the
                // shift right, since an immediate may have bit 30 set.
                0x13 => {
                    let alt = alt.and(eq(f3, U::from(5u8)));
                    write = Some(alu(f3, alt, a, imm_i));
                }
                0x33 => write = Some(alu(f3, alt, a, b)),
                0x0f => {}
                _ => {
                    // ecall, ebreak, and anything the core does not know.
                    next = pc;
                    stop = Bit::One;
                }
            }
            let wrote = write.is_some() && rd.raw() != 0;
            if let Some(v) = write {
                if wrote {
                    self.regs.write(rd.raw() as usize, v);
                }
            }
            // A redirect: the next instruction is not the one the
            // fetch stage read this cycle, so that word is squashed
            // and the fetch restarts at the target.
            let redirect = run && next != pc4;
            if rst.to_bool() {
                self.pc.set(U::from(0u8));
                self.valid.set(Bit::Zero);
            } else if stop.to_bool() {
                // Halted: the program counter parks on the halting
                // instruction, as the model's does.
                if run {
                    self.pc.set(pc);
                }
                self.valid.set(Bit::Zero);
            } else if redirect {
                self.pc.set(next);
                self.valid.set(Bit::Zero);
            } else {
                self.pc.set(fetch_pc.wrapping_add(U::from(4u8)));
                self.ir.set(fetched);
                self.ir_pc.set(fetch_pc);
                self.valid.set(Bit::One);
            }
            self.halted.set(stop);
            halt.set(stop);
            instr.set(if run { ir } else { U::from(0u8) });
            wb.set(Writeback {
                done: Bit::from_bool(run),
                rd: if wrote { rd } else { U::from(0u8) },
                val: write.unwrap_or_default(),
            });
        }
    }
}
