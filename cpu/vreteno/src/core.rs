// SPDX-License-Identifier: Apache-2.0
//! Vreteno: a two-stage RV32I core. One process, and on every edge
//! two things at once: the fetch stage reads the instruction memory
//! at the program counter into the instruction register, and the
//! execute stage decodes the fields of the word in that register,
//! executes, writes the register file and the data memory, and, on a
//! taken branch or a jump, redirects the fetch and squashes the word
//! it fetched this cycle, which is the one-cycle penalty.
//!
//! Written in the subset `#[lower]` reads: every value is a function
//! of the state and the inputs, `select!` chooses among values and
//! `when!` and `case!` among drives, and nothing branches. So the
//! same file simulates and lowers, and the netlist is simulated
//! against the trace the simulation wrote.
use txhdl::comp::{mux, Clock, DefaultClock, In, Mem, Out, Reg, Unit};
use txhdl::funcs::{
    band, bor, bxor, eq, is_zero, lt, lt_signed, shl, shr, sra,
};
use txhdl::types::{Bit, U};
use txhdl::{case, lower, select, when, Trace, Value};

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

impl Vreteno {
    /// A core with its program loaded.
    pub fn with(program: &[u32]) -> Self {
        let words: Vec<U<32>> = program.iter().map(|&w| U::from(w)).collect();
        Vreteno {
            imem: Mem::with(&words),
            ..Default::default()
        }
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

#[lower]
impl Unit<In<Bit>, (Out<Bit>, Out<U<32>>, Out<Writeback>)> for Vreteno {
    async fn run(
        &mut self,
        rst: In<Bit>,
        (halt, instr, wb): (Out<Bit>, Out<U<32>>, Out<Writeback>),
    ) {
        loop {
            DefaultClock::rising().await;
            let rst = rst.get();
            let (fetch_pc, halted) = (self.pc.get(), self.halted.get());
            let (ir, pc, valid) =
                (self.ir.get(), self.ir_pc.get(), self.valid.get());
            // The fetch stage: the word at the program counter, into
            // the instruction register unless the execute stage
            // redirects below.
            let fetched = self.imem.read(fetch_pc.slice::<2, 10>());
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
            // The operands: register zero reads as zero.
            let a = mux(is_zero(rs1), U::from(0u32), self.regs.read(rs1));
            let b = mux(is_zero(rs2), U::from(0u32), self.regs.read(rs2));
            let pc4 = pc.wrapping_add(U::from(4u8));
            let run = rst.not().and(halted.not()).and(valid);

            // The ALU, shared by the register and immediate forms; bit
            // 30 means subtract or arithmetic shift, except that an
            // immediate may have it set and mean nothing by it.
            let alu_b = mux(eq(opcode, U::from(0x13u8)), imm_i, b);
            let sh = alu_b.slice::<0, 5>();
            let sub =
                alt.and(eq(opcode, U::from(0x33u8)).or(eq(f3, U::from(5u8))));
            let alu = select!(f3.raw() => {
                0 => mux(sub, a.wrapping_sub(alu_b), a.wrapping_add(alu_b)),
                1 => shl(a, sh.raw() as usize),
                2 => lt_signed(a, alu_b).zext(),
                3 => lt(a, alu_b).zext(),
                4 => bxor(a, alu_b),
                5 => mux(
                    sub,
                    sra(a, sh.raw() as usize),
                    shr(a, sh.raw() as usize)
                ),
                6 => bor(a, alu_b),
                _ => band(a, alu_b),
            });
            // The branch condition.
            let taken = select!(f3.raw() => {
                0 => eq(a, b),
                1 => eq(a, b).not(),
                4 => lt_signed(a, b),
                5 => lt_signed(a, b).not(),
                6 => lt(a, b),
                _ => lt(a, b).not(),
            });
            // Loads and stores: the word, the lane within it, and the
            // byte or half in that lane, extended or merged.
            let addr = a.wrapping_add(select!(opcode.raw() => {
                0x23 => imm_s,
                _ => imm_i,
            }));
            let daddr = addr.wrapping_sub(U::from(DATA_BASE)).slice::<2, 10>();
            let word = self.dmem.read(daddr);
            let bsh = addr.slice::<0, 2>().concat::<3, 5>(U::<3>::from(0u8));
            let hsh = addr.slice::<1, 1>().concat::<4, 5>(U::<4>::from(0u8));
            let octet = shr(word, bsh.raw() as usize).slice::<0, 8>();
            let half = shr(word, hsh.raw() as usize).slice::<0, 16>();
            let loaded = select!(f3.raw() => {
                0 => octet.sext::<32>(),
                1 => half.sext::<32>(),
                2 => word,
                4 => octet.zext::<32>(),
                _ => half.zext::<32>(),
            });
            let bmask = shl(U::<32>::from(0xffu32), bsh.raw() as usize);
            let hmask = shl(U::<32>::from(0xffffu32), hsh.raw() as usize);
            let stored = select!(f3.raw() => {
                0 => bor(
                    band(word, bmask.not()),
                    shl(b.slice::<0, 8>().zext::<32>(), bsh.raw() as usize)
                ),
                1 => bor(
                    band(word, hmask.not()),
                    shl(b.slice::<0, 16>().zext::<32>(), hsh.raw() as usize)
                ),
                _ => b,
            });

            // What the instruction does: the value it writes back, if
            // any, where it goes next, and whether the core knows it.
            let writes = select!(opcode.raw() => {
                0x37 | 0x17 | 0x6f | 0x67 | 0x03 | 0x13 | 0x33 => Bit::One,
                _ => Bit::Zero,
            });
            let wval = select!(opcode.raw() => {
                0x37 => imm_u,
                0x17 => pc.wrapping_add(imm_u),
                0x6f | 0x67 => pc4,
                0x03 => loaded,
                _ => alu,
            });
            let target = select!(opcode.raw() => {
                0x6f => pc.wrapping_add(imm_j),
                0x67 => a.wrapping_add(imm_i).and(U::<32>::from(1u32).not()),
                _ => pc.wrapping_add(imm_b),
            });
            let jump = select!(opcode.raw() => {
                0x6f | 0x67 => Bit::One,
                0x63 => taken,
                _ => Bit::Zero,
            });
            let known = select!(opcode.raw() => {
                0x37 | 0x17 | 0x6f | 0x67 | 0x63 | 0x03 | 0x23 | 0x13 | 0x33
                | 0x0f => Bit::One,
                _ => Bit::Zero,
            });
            // Halted: on ebreak, ecall or a word the core does not
            // know, and then for good.
            let stop = mux(run, known.not(), halted);
            let wrote = run.and(writes).and(is_zero(rd).not());
            let store = run.and(eq(opcode, U::from(0x23u8)));

            // The drives. A redirect means the next instruction is not
            // the one the fetch stage read this cycle, so that word is
            // squashed and the fetch restarts at the target; a halt
            // parks the program counter on the halting instruction, as
            // the model's does.
            when!(wrote => { self.regs.at(rd) <= wval });
            when!(store => { self.dmem.at(daddr) <= stored });
            case!(rst => {
                Bit::One => {
                    self.pc <= U::from(0u8);
                    self.valid <= Bit::Zero
                },
                _ if run.and(stop).to_bool() => {
                    self.pc <= pc;
                    self.valid <= Bit::Zero
                },
                _ if stop.to_bool() => { self.valid <= Bit::Zero },
                _ if run.and(jump).to_bool() => {
                    self.pc <= target;
                    self.valid <= Bit::Zero
                },
                _ => {
                    self.pc <= fetch_pc.wrapping_add(U::from(4u8));
                    self.ir <= fetched;
                    self.ir_pc <= fetch_pc;
                    self.valid <= Bit::One
                },
            });
            self.halted.set(stop);
            halt.set(stop);
            instr.set(mux(run, ir, U::from(0u32)));
            wb.set(Writeback {
                done: run,
                rd: mux(wrote, rd, U::from(0u8)),
                val: mux(run.and(writes), wval, U::from(0u32)),
            });
        }
    }
}
