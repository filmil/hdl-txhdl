// SPDX-License-Identifier: Apache-2.0
//! Vreteno: a three-stage RV32I core. One process, and on every edge
//! three things at once: the fetch stage reads the instruction memory
//! at the program counter into the instruction register; the execute
//! stage decodes the fields of the word in that register, executes,
//! stores, reads the data memory into a register at its edge, and, on
//! a taken branch or a jump, redirects the fetch and squashes the word
//! it fetched this cycle, which is the one-cycle penalty; and the
//! writeback stage extends a loaded word, writes the register file and
//! retires. An instruction in execute that reads what the one in
//! writeback has not yet written is given that value directly, the
//! forwarding path, so no stall is ever needed.
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

/// The data memory is four memories of a byte, one per lane of the
/// word, each with a write port of its own, so a store of a byte or a
/// half writes its lanes and reads nothing, and the one read of the
/// four lanes feeds a register: what a block RAM is.
///
/// The state. The fetch stage's program counter. The instruction
/// register, its program counter and whether it holds an instruction,
/// the boundary between fetch and execute. The writeback registers,
/// the boundary between execute and writeback: whether one is there,
/// its destination, its value if not a load, the raw word read for a
/// load with the lane and the width to take from it, and whether it
/// halts. The halt itself, which the writeback stage sets. And the
/// three memories.
#[derive(Trace, Default)]
pub struct Vreteno {
    pub pc: Reg<U<32>>,
    pub ir: Reg<U<32>>,
    pub ir_pc: Reg<U<32>>,
    pub valid: Reg<Bit>,
    pub stopped: Reg<Bit>,
    pub wb_valid: Reg<Bit>,
    pub wb_pc: Reg<U<32>>,
    pub wb_rd: Reg<U<5>>,
    pub wb_alu: Reg<U<32>>,
    pub wb_ld: Reg<U<32>>,
    pub wb_f3: Reg<U<3>>,
    pub wb_lane: Reg<U<2>>,
    pub wb_load: Reg<Bit>,
    pub wb_stop: Reg<Bit>,
    pub halted: Reg<Bit>,
    pub regs: Mem<U<32>, 32>,
    pub imem: Mem<U<32>, IMEM_WORDS>,
    pub dmem0: Mem<U<8>, DMEM_WORDS>,
    pub dmem1: Mem<U<8>, DMEM_WORDS>,
    pub dmem2: Mem<U<8>, DMEM_WORDS>,
    pub dmem3: Mem<U<8>, DMEM_WORDS>,
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

    /// The architectural program counter: that of the oldest
    /// instruction not yet retired, in writeback, else in execute,
    /// else the fetch's. What the model's program counter is compared
    /// against.
    pub fn data_word(&self, at: usize) -> u32 {
        let lane = |m: &Mem<U<8>, DMEM_WORDS>| m.read(at).raw() as u32;
        lane(&self.dmem0)
            | lane(&self.dmem1) << 8
            | lane(&self.dmem2) << 16
            | lane(&self.dmem3) << 24
    }

    pub fn arch_pc(&self) -> U<32> {
        if self.wb_valid.get().to_bool() {
            self.wb_pc.get()
        } else if self.valid.get().to_bool() {
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
            let (fetch_pc, stopped) = (self.pc.get(), self.stopped.get());
            let (ir, pc, valid) =
                (self.ir.get(), self.ir_pc.get(), self.valid.get());
            let (wb_valid, wb_rd, wb_alu) =
                (self.wb_valid.get(), self.wb_rd.get(), self.wb_alu.get());
            let (wb_ld, wb_f3, wb_lane) =
                (self.wb_ld.get(), self.wb_f3.get(), self.wb_lane.get());
            let (wb_load, wb_stop, halted) =
                (self.wb_load.get(), self.wb_stop.get(), self.halted.get());
            // The writeback stage: a loaded word's byte or half, by the
            // lane and the width the execute stage read with it,
            // extended; else the value execute computed. Written to the
            // register file, and forwarded to execute below.
            let ld_bsh = wb_lane.concat::<3, 5>(U::<3>::from(0u8));
            let ld_hsh =
                wb_lane.slice::<1, 1>().concat::<4, 5>(U::<4>::from(0u8));
            let ld_octet = shr(wb_ld, ld_bsh.raw() as usize).slice::<0, 8>();
            let ld_half = shr(wb_ld, ld_hsh.raw() as usize).slice::<0, 16>();
            let loaded = select!(wb_f3.raw() => {
                0 => ld_octet.sext::<32>(),
                1 => ld_half.sext::<32>(),
                2 => wb_ld,
                4 => ld_octet.zext::<32>(),
                _ => ld_half.zext::<32>(),
            });
            let wb_val = mux(wb_load, loaded, wb_alu);
            let wb_write = wb_valid.and(is_zero(wb_rd).not());
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
            // The operands: register zero reads as zero, and a register
            // the writeback stage is about to write reads as the value it
            // will write, which is the forwarding path.
            let fwd_a = wb_write.and(eq(wb_rd, rs1));
            let fwd_b = wb_write.and(eq(wb_rd, rs2));
            let a = mux(
                is_zero(rs1),
                U::from(0u32),
                mux(fwd_a, wb_val, self.regs.read(rs1)),
            );
            let b = mux(
                is_zero(rs2),
                U::from(0u32),
                mux(fwd_b, wb_val, self.regs.read(rs2)),
            );
            let pc4 = pc.wrapping_add(U::from(4u8));
            let run = rst.not().and(stopped.not()).and(valid);

            // The ALU, shared by the register and immediate forms; bit
            // 30 means subtract or arithmetic shift, except that an
            // immediate may have it set and mean nothing by it.
            let alu_b = mux(eq(opcode, U::from(0x13u8)), imm_i, b);
            let sh = alu_b.slice::<0, 5>();
            let sub =
                alt.and(eq(opcode, U::from(0x33u8)).or(eq(f3, U::from(5u8))));
            // begin{alu}
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
            // end{alu}
            // The branch condition.
            let taken = select!(f3.raw() => {
                0 => eq(a, b),
                1 => eq(a, b).not(),
                4 => lt_signed(a, b),
                5 => lt_signed(a, b).not(),
                6 => lt(a, b),
                _ => lt(a, b).not(),
            });
            // Loads and stores: the word, the lane within it. A load
            // takes the raw word into the writeback stage, which is the
            // synchronous read a block RAM has; a store merges its byte
            // or half into the word and writes it here.
            let addr = a.wrapping_add(select!(opcode.raw() => {
                0x23 => imm_s,
                _ => imm_i,
            }));
            let daddr = addr.wrapping_sub(U::from(DATA_BASE)).slice::<2, 10>();
            let word = self
                .dmem3
                .read(daddr)
                .concat::<8, 16>(self.dmem2.read(daddr))
                .concat::<8, 24>(self.dmem1.read(daddr))
                .concat::<8, 32>(self.dmem0.read(daddr));
            let lane = addr.slice::<0, 2>();
            let upper = addr.bit(1);
            // What each lane takes on a store: its byte of the word for
            // a word, the half's byte for a half, the byte for a byte;
            // and whether it takes it at all.
            let (b0, b1) = (b.slice::<0, 8>(), b.slice::<8, 8>());
            let (b2, b3) = (b.slice::<16, 8>(), b.slice::<24, 8>());
            let d1 = select!(f3.raw() => { 0 => b0, _ => b1 });
            let d2 = select!(f3.raw() => { 2 => b2, _ => b0 });
            let d3 = select!(f3.raw() => { 0 => b0, 2 => b3, _ => b1 });
            let en0 = select!(f3.raw() => {
                0 => eq(lane, U::from(0u8)),
                1 => upper.not(),
                _ => Bit::One,
            });
            let en1 = select!(f3.raw() => {
                0 => eq(lane, U::from(1u8)),
                1 => upper.not(),
                _ => Bit::One,
            });
            let en2 = select!(f3.raw() => {
                0 => eq(lane, U::from(2u8)),
                1 => upper,
                _ => Bit::One,
            });
            let en3 = select!(f3.raw() => {
                0 => eq(lane, U::from(3u8)),
                1 => upper,
                _ => Bit::One,
            });

            // What the instruction does: the value it writes back, if
            // any, where it goes next, and whether the core knows it.
            let writes = select!(opcode.raw() => {
                0x37 | 0x17 | 0x6f | 0x67 | 0x03 | 0x13 | 0x33 => Bit::One,
                _ => Bit::Zero,
            });
            let is_load = eq(opcode, U::from(0x03u8));
            let wval = select!(opcode.raw() => {
                0x37 => imm_u,
                0x17 => pc.wrapping_add(imm_u),
                0x6f | 0x67 => pc4,
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
            // Stopped: on ebreak, ecall or a word the core does not
            // know, and then for good; the halt itself follows a cycle
            // later, when the halting instruction retires.
            let stop = mux(run, known.not(), stopped);
            let wrote = run.and(writes).and(is_zero(rd).not());
            let store = run.and(eq(opcode, U::from(0x23u8)));

            // The drives. The writeback stage writes the register file
            // and sets the halt. Execute hands the writeback stage what
            // it needs, or nothing. A redirect means the next
            // instruction is not the one the fetch stage read this
            // cycle, so that word is squashed and the fetch restarts at
            // the target; a halt parks the program counter on the
            // halting instruction, as the model's does.
            when!(wb_write => { self.regs.at(wb_rd) <= wb_val });
            self.halted.set(halted.or(wb_stop));
            when!(store.and(en0) => { self.dmem0.at(daddr) <= b0 });
            when!(store.and(en1) => { self.dmem1.at(daddr) <= d1 });
            when!(store.and(en2) => { self.dmem2.at(daddr) <= d2 });
            when!(store.and(en3) => { self.dmem3.at(daddr) <= d3 });
            when!(run => {
                self.wb_valid <= Bit::One;
                self.wb_pc <= pc;
                self.wb_rd <= mux(wrote, rd, U::from(0u8));
                self.wb_alu <= wval;
                self.wb_ld <= word;
                self.wb_f3 <= f3;
                self.wb_lane <= lane;
                self.wb_load <= is_load;
                self.wb_stop <= known.not()
            } else {
                self.wb_valid <= Bit::Zero;
                self.wb_rd <= U::from(0u8);
                self.wb_stop <= Bit::Zero
            });
            // begin{fetch}
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
            // end{fetch}
            self.stopped.set(stop);
            halt.set(halted);
            instr.set(mux(run, ir, U::from(0u32)));
            wb.set(Writeback {
                done: wb_valid,
                rd: wb_rd,
                val: mux(wb_valid, wb_val, U::from(0u32)),
            });
        }
    }
}
