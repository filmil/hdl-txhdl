// SPDX-License-Identifier: Apache-2.0
//! Vreteno: a three-stage RV32IM core. One process, and on every edge
//! three things at once: the fetch stage reads the instruction memory
//! at the program counter into the instruction register; the execute
//! stage decodes the fields of the word in that register, executes,
//! stores, reads the data memory into a register at its edge, and, on
//! a taken branch or a jump, redirects the fetch and squashes the word
//! it fetched this cycle, which is the one-cycle penalty; and the
//! writeback stage extends a loaded word, writes the register file and
//! retires. An instruction in execute that reads what the one in
//! writeback has not yet written is given that value directly, the
//! forwarding path, unless that value is a load's, which lands too
//! late to forward: then the instruction waits one cycle, the one
//! stall there is. A trap or an `mret` is a redirect like a jump's.
//!
//! Written in the subset `#[lower]` reads: every value is a function
//! of the state and the inputs, `select!` chooses among values and
//! `when!` and `case!` among drives, and nothing branches. So the
//! same file simulates and lowers, and the netlist is simulated
//! against the trace the simulation wrote.
use crate::isa::{
    CAUSE_ECALL, CAUSE_ILLEGAL, CAUSE_MEXT, CAUSE_MTIMER, CSR_MCAUSE, CSR_MEPC,
    CSR_MIE, CSR_MIP, CSR_MSCRATCH, CSR_MSTATUS, CSR_MTVAL, CSR_MTVEC, MEXT,
    MTIMER, TIMER_BASE,
};
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
    pub wb_ir: Reg<U<32>>,
    pub wb_rd: Reg<U<5>>,
    pub wb_alu: Reg<U<32>>,
    pub wb_ld: Reg<U<32>>,
    pub wb_f3: Reg<U<3>>,
    pub wb_lane: Reg<U<2>>,
    pub wb_load: Reg<Bit>,
    pub wb_stop: Reg<Bit>,
    pub halted: Reg<Bit>,
    pub mstatus: Reg<U<32>>,
    pub mtvec: Reg<U<32>>,
    pub mscratch: Reg<U<32>>,
    pub mepc: Reg<U<32>>,
    pub mcause: Reg<U<32>>,
    pub mie: Reg<U<32>>,
    pub mip: Reg<U<32>>,
    pub mtval: Reg<U<32>>,
    pub mtime: Reg<U<64>>,
    pub mtimecmp: Reg<U<64>>,
    pub wb_dev: Reg<U<32>>,
    pub wb_is_dev: Reg<Bit>,
    pub m_busy: Reg<Bit>,
    pub m_count: Reg<U<6>>,
    pub m_hi: Reg<U<33>>,
    pub m_lo: Reg<U<32>>,
    pub m_d: Reg<U<32>>,
    pub m_neg_q: Reg<Bit>,
    pub m_neg_r: Reg<Bit>,
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
impl Unit<(In<Bit>, In<Bit>), (Out<Bit>, Out<U<32>>, Out<Writeback>)>
    for Vreteno
{
    async fn run(
        &mut self,
        (rst, irq): (In<Bit>, In<Bit>),
        (halt, instr, wb): (Out<Bit>, Out<U<32>>, Out<Writeback>),
    ) {
        loop {
            DefaultClock::rising().await;
            let (rst, irq) = (rst.get(), irq.get());
            let (fetch_pc, stopped) = (self.pc.get(), self.stopped.get());
            let (ir, pc, valid) =
                (self.ir.get(), self.ir_pc.get(), self.valid.get());
            let (wb_valid, wb_rd, wb_alu) =
                (self.wb_valid.get(), self.wb_rd.get(), self.wb_alu.get());
            let wb_ir = self.wb_ir.get();
            let (wb_ld, wb_f3, wb_lane) =
                (self.wb_ld.get(), self.wb_f3.get(), self.wb_lane.get());
            let (wb_load, wb_stop, halted) =
                (self.wb_load.get(), self.wb_stop.get(), self.halted.get());
            let (mstatus, mtvec, mscratch) =
                (self.mstatus.get(), self.mtvec.get(), self.mscratch.get());
            let (mepc, mcause) = (self.mepc.get(), self.mcause.get());
            let (mie_r, mip, mtval) =
                (self.mie.get(), self.mip.get(), self.mtval.get());
            let (mtime, mtimecmp) = (self.mtime.get(), self.mtimecmp.get());
            let (wb_dev, wb_is_dev) = (self.wb_dev.get(), self.wb_is_dev.get());
            let (m_busy, m_count) = (self.m_busy.get(), self.m_count.get());
            let (m_hi, m_lo, m_d) =
                (self.m_hi.get(), self.m_lo.get(), self.m_d.get());
            let (m_neg_q, m_neg_r) = (self.m_neg_q.get(), self.m_neg_r.get());
            // The writeback stage: a loaded word's byte or half, by the
            // lane and the width the execute stage read with it,
            // extended; else the value execute computed. Written to the
            // register file, and forwarded to execute below.
            let ld_bsh = wb_lane.concat::<3, 5>(U::<3>::from(0u8));
            let ld_hsh =
                wb_lane.slice::<1, 1>().concat::<4, 5>(U::<4>::from(0u8));
            // A load's word: the data memory's, read into its register at
            // the edge, or the timer's, read into another, so that the
            // memory's register stays the block RAM's own.
            let wb_src = mux(wb_is_dev, wb_dev, wb_ld);
            let ld_octet = shr(wb_src, ld_bsh.raw() as usize).slice::<0, 8>();
            let ld_half = shr(wb_src, ld_hsh.raw() as usize).slice::<0, 16>();
            let loaded = select!(wb_f3.raw() => {
                0 => ld_octet.sext::<32>(),
                1 => ld_half.sext::<32>(),
                2 => wb_src,
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
            // will write, which is the forwarding path. A load is the
            // exception: its word lands at the edge and is extended in
            // writeback, too long a path to forward through the ALU into
            // the fetch, so an instruction that reads what the load in
            // writeback will write waits one cycle and reads the register
            // file, which has it by then. The stall is the one cycle the
            // core ever waits.
            let fwd_a = wb_write.and(eq(wb_rd, rs1));
            let fwd_b = wb_write.and(eq(wb_rd, rs2));
            let stall_ld = valid.and(wb_load).and(fwd_a.or(fwd_b));
            // The M extension is a sequencer: a multiply is one step in
            // the part's multipliers, a division thirty-two restoring
            // steps, over the magnitudes, and the instruction stalls in
            // execute until the count is up. One set of registers serves
            // both.
            let f7 = ir.slice::<25, 7>();
            let is_m = eq(opcode, U::from(0x33u8)).and(eq(f7, U::from(1u8)));
            let m_here = valid.and(rst.not()).and(stopped.not()).and(is_m);
            let m_done = m_busy.and(eq(m_count, U::from(32u8)));
            // An interrupt is taken instead of the instruction in execute
            // when it is pending, enabled, and interrupts are enabled; an
            // M instruction under one does not start, so it cannot hold
            // the stall the interrupt waits for.
            let mtip = lt(mtime, mtimecmp).not();
            let ext_ok = mie_r.bit(11).and(mip.bit(11));
            let tim_ok = mie_r.bit(7).and(mtip);
            let int_ok = mstatus.bit(3).and(ext_ok.or(tim_ok));
            let stall_m = m_here.and(int_ok.not()).and(m_done.not());
            let stall = stall_ld.or(stall_m);
            let a = mux(
                is_zero(rs1),
                U::<32>::from(0u32),
                mux(fwd_a, wb_alu, self.regs.read(rs1)),
            );
            let b = mux(
                is_zero(rs2),
                U::<32>::from(0u32),
                mux(fwd_b, wb_alu, self.regs.read(rs2)),
            );
            let pc4 = pc.wrapping_add(U::from(4u8));
            // Live: an instruction in execute that is not stalled. It
            // runs unless the interrupt takes its place.
            let live = rst.not().and(stopped.not()).and(valid).and(stall.not());
            let int_take = live.and(int_ok);
            let run = live.and(int_take.not());

            // The ALU, shared by the register and immediate forms; bit
            // 30 means subtract or arithmetic shift, except that an
            // immediate may have it set and mean nothing by it.
            let alu_b = mux(eq(opcode, U::from(0x13u8)), imm_i, b);
            let sh = alu_b.slice::<0, 5>();
            let sub =
                alt.and(eq(opcode, U::from(0x33u8)).or(eq(f3, U::from(5u8))));
            // The multiply and divide, from the sequencer's registers,
            // with the signs put back: a product or a quotient is
            // negated when the signs differed, a remainder takes the
            // dividend's sign, and a quotient by zero is all ones.
            let m_mag = m_hi.slice::<0, 32>().concat::<32, 64>(m_lo);
            let m_p =
                mux(m_neg_q, U::<64>::from(0u32).wrapping_sub(m_mag), m_mag);
            let m_q =
                mux(m_neg_q, U::<32>::from(0u32).wrapping_sub(m_lo), m_lo);
            let m_rem = m_hi.slice::<0, 32>();
            let m_r =
                mux(m_neg_r, U::<32>::from(0u32).wrapping_sub(m_rem), m_rem);
            let m_res = select!(f3.raw() => {
                0 => m_p.slice::<0, 32>(),
                1 | 2 | 3 => m_p.slice::<32, 32>(),
                4 | 5 => m_q,
                _ => m_r,
            });
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
            // The timer: four words at TIMER_BASE, the count and the
            // compare, low half then high. A load reads one, a store
            // writes one through the lanes as a memory word is written,
            // and the count runs from the reset.
            let is_dev = eq(addr.slice::<4, 28>(), U::from(TIMER_BASE >> 4));
            let dev_sel = addr.slice::<2, 2>();
            let dev_word = select!(dev_sel.raw() => {
                0 => mtime.slice::<0, 32>(),
                1 => mtime.slice::<32, 32>(),
                2 => mtimecmp.slice::<0, 32>(),
                _ => mtimecmp.slice::<32, 32>(),
            });
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
                0x73 => is_zero(f3).not(),
                _ => Bit::Zero,
            });
            let is_load = eq(opcode, U::from(0x03u8));
            // The system instructions. A CSR instruction reads one of
            // the five registers and writes it, set or cleared or
            // replaced, from a register or a five-bit immediate; ecall
            // and an instruction the core does not know trap; mret
            // returns; ebreak halts.
            let f12 = ir.slice::<20, 12>();
            let is_sys = eq(opcode, U::from(0x73u8));
            let csr_op = is_sys.and(is_zero(f3).not());
            let csr_old = select!(f12.raw() => {
                0x300 => mstatus,
                0x305 => mtvec,
                0x340 => mscratch,
                0x341 => mepc,
                0x342 => mcause,
                0x304 => mie_r,
                0x344 => mux(mtip, bor(mip, U::<32>::from(MTIMER)), mip),
                0x343 => mtval,
                _ => U::<32>::from(0u32),
            });
            let csr_known = select!(f12.raw() => {
                0x300 | 0x305 | 0x340 | 0x341 | 0x342 | 0x304 | 0x344
                | 0x343 => Bit::One,
                _ => Bit::Zero,
            });
            let csr_src = mux(f3.bit(2), rs1.zext::<32>(), a);
            let csr_new = select!(f3.raw() => {
                1 | 5 => csr_src,
                2 | 6 => bor(csr_old, csr_src),
                _ => band(csr_old, csr_src.not()),
            });
            let sys0 = is_sys.and(is_zero(f3));
            let is_ecall = sys0.and(eq(f12, U::from(0u8)));
            let is_ebreak = sys0.and(eq(f12, U::from(1u8)));
            let is_mret = sys0.and(eq(f12, U::from(0x302u32)));
            let known = select!(opcode.raw() => {
                0x37 | 0x17 | 0x6f | 0x67 | 0x63 | 0x03 | 0x23 | 0x13 | 0x33
                | 0x0f => Bit::One,
                0x73 => csr_op.and(csr_known).or(is_ecall).or(is_ebreak)
                    .or(is_mret),
                _ => Bit::Zero,
            });
            // A trap: ecall, a word the core does not know, or the
            // interrupt. The cause, the address and the trap value go to
            // the CSRs, the interrupt enable is saved and cleared, and
            // the handler is the redirect.
            let trap = run.and(is_ecall.or(known.not())).or(int_take);
            let cause = mux(
                int_take,
                mux(
                    ext_ok,
                    U::<32>::from(CAUSE_MEXT),
                    U::<32>::from(CAUSE_MTIMER),
                ),
                mux(
                    is_ecall,
                    U::<32>::from(CAUSE_ECALL),
                    U::<32>::from(CAUSE_ILLEGAL),
                ),
            );
            let tval = mux(run.and(known.not()), ir, U::<32>::from(0u32));
            let mie_bit = mstatus.bit(3);
            let mpie = mstatus.bit(7);
            let trap_status =
                mux(mie_bit, U::<32>::from(0x80u32), U::<32>::from(0u32));
            let mret_status =
                mux(mpie, U::<32>::from(0x88u32), U::<32>::from(0x80u32));
            let csr_write = run.and(csr_op).and(csr_known);
            // Stopped: on ebreak, and then for good; the halt itself
            // follows a cycle later, when the halting instruction
            // retires.
            let stop = mux(run, is_ebreak, stopped);
            let wrote = run.and(writes).and(is_zero(rd).not()).and(trap.not());
            let store = run.and(eq(opcode, U::from(0x23u8)));
            let wval = select!(opcode.raw() => {
                0x37 => imm_u,
                0x17 => pc.wrapping_add(imm_u),
                0x6f | 0x67 => pc4,
                0x73 => csr_old,
                _ => mux(is_m, m_res, alu),
            });
            // Where the instruction goes next, other than on: a jump's
            // or a taken branch's target, the saved address on mret, the
            // handler on a trap, which is every arm that can trap. The
            // jalr arm is first because it is last to settle: a loaded
            // value forwarded into the add, and this select is on the
            // critical path.
            let target = select!(opcode.raw() => {
                0x67 => a.wrapping_add(imm_i).and(U::<32>::from(1u32).not()),
                0x6f => pc.wrapping_add(imm_j),
                0x63 => pc.wrapping_add(imm_b),
                0x73 => mux(is_mret, mepc, mtvec),
                _ => mtvec,
            });
            let jump = select!(opcode.raw() => {
                0x6f | 0x67 => Bit::One,
                0x63 => taken,
                0x73 => is_mret.or(trap),
                _ => trap,
            });

            // The drives. The writeback stage writes the register file
            // and sets the halt. Execute hands the writeback stage what
            // it needs, or nothing. A redirect means the next
            // instruction is not the one the fetch stage read this
            // cycle, so that word is squashed and the fetch restarts at
            // the target; a halt parks the program counter on the
            // halting instruction, as the model's does. The redirect is
            // one mux on the way into the fetch's state, so the target,
            // the last value to settle, passes through as little as
            // possible; the word fetched under a redirect is written
            // and marked empty.
            when!(wb_write => { self.regs.at(wb_rd) <= wb_val });
            self.halted.set(halted.or(wb_stop));
            let store_mem = store.and(is_dev.not());
            when!(store_mem.and(en0) => { self.dmem0.at(daddr) <= b0 });
            when!(store_mem.and(en1) => { self.dmem1.at(daddr) <= d1 });
            when!(store_mem.and(en2) => { self.dmem2.at(daddr) <= d2 });
            when!(store_mem.and(en3) => { self.dmem3.at(daddr) <= d3 });
            // The timer counts, and a store to one of its words puts the
            // lanes the store covers into that word.
            let dev_new = mux(en3, d3, dev_word.slice::<24, 8>())
                .concat::<8, 16>(mux(en2, d2, dev_word.slice::<16, 8>()))
                .concat::<8, 24>(mux(en1, d1, dev_word.slice::<8, 8>()))
                .concat::<8, 32>(mux(en0, b0, dev_word.slice::<0, 8>()));
            let store_dev = store.and(is_dev);
            self.mtime.set(mux(
                rst,
                U::<64>::from(0u32),
                mtime.wrapping_add(U::<64>::from(1u32)),
            ));
            when!(store_dev.and(eq(dev_sel, U::from(0u8))) => {
                self.mtime <= mtime.slice::<32, 32>().concat::<32, 64>(dev_new)
            });
            when!(store_dev.and(eq(dev_sel, U::from(1u8))) => {
                self.mtime <= dev_new.concat::<32, 64>(mtime.slice::<0, 32>())
            });
            when!(store_dev.and(eq(dev_sel, U::from(2u8))) => {
                self.mtimecmp <=
                    mtimecmp.slice::<32, 32>().concat::<32, 64>(dev_new)
            });
            when!(store_dev.and(eq(dev_sel, U::from(3u8))) => {
                self.mtimecmp <=
                    dev_new.concat::<32, 64>(mtimecmp.slice::<0, 32>())
            });
            // The CSRs: written by a CSR instruction, by a trap, by mret.
            // The three never coincide in one instruction.
            when!(csr_write.and(eq(f12, U::from(CSR_MSTATUS))) => {
                self.mstatus <= band(csr_new, U::<32>::from(0x88u32))
            });
            when!(csr_write.and(eq(f12, U::from(CSR_MTVEC))) => {
                self.mtvec <= band(csr_new, U::<32>::from(3u32).not())
            });
            when!(csr_write.and(eq(f12, U::from(CSR_MSCRATCH))) => {
                self.mscratch <= csr_new
            });
            when!(csr_write.and(eq(f12, U::from(CSR_MEPC))) => {
                self.mepc <= band(csr_new, U::<32>::from(1u32).not())
            });
            when!(csr_write.and(eq(f12, U::from(CSR_MCAUSE))) => {
                self.mcause <= csr_new
            });
            when!(csr_write.and(eq(f12, U::from(CSR_MIE))) => {
                self.mie <= band(csr_new, U::<32>::from(MEXT | MTIMER))
            });
            when!(csr_write.and(eq(f12, U::from(CSR_MTVAL))) => {
                self.mtval <= csr_new
            });
            // The pending bit: set by the line, cleared by software,
            // and the write wins when both fall in one cycle.
            when!(irq => { self.mip <= bor(mip, U::<32>::from(MEXT)) });
            when!(csr_write.and(eq(f12, U::from(CSR_MIP))) => {
                self.mip <= band(csr_new, U::<32>::from(MEXT))
            });
            when!(trap => {
                self.mepc <= pc;
                self.mcause <= cause;
                self.mtval <= tval;
                self.mstatus <= trap_status
            });
            when!(run.and(is_mret) => { self.mstatus <= mret_status });
            // The sequencer. It starts when an M instruction is in execute
            // with its operands ready, and is released when the
            // instruction runs. A multiply is one step: the product of
            // the magnitudes, from the registers into the pair, which the
            // part's DSP blocks make; a division is thirty-two, each
            // shifting the pair left, subtracting the divisor from the
            // high half when it fits, and shifting the fit in as the
            // quotient bit.
            let m_signed_a = eq(f3, U::from(0u8))
                .or(eq(f3, U::from(1u8)))
                .or(eq(f3, U::from(2u8)))
                .or(eq(f3, U::from(4u8)))
                .or(eq(f3, U::from(6u8)));
            let m_signed_b = eq(f3, U::from(0u8))
                .or(eq(f3, U::from(1u8)))
                .or(eq(f3, U::from(4u8)))
                .or(eq(f3, U::from(6u8)));
            let m_neg_a = m_signed_a.and(a.bit(31));
            let m_neg_b = m_signed_b.and(b.bit(31));
            let m_abs_a = mux(m_neg_a, U::<32>::from(0u32).wrapping_sub(a), a);
            let m_abs_b = mux(m_neg_b, U::<32>::from(0u32).wrapping_sub(b), b);
            let m_differ =
                m_neg_a.and(m_neg_b.not()).or(m_neg_a.not().and(m_neg_b));
            let m_is_div = f3.bit(2);
            let m_start = m_here
                .and(m_busy.not())
                .and(stall_ld.not())
                .and(int_ok.not());
            let m_step = m_busy.and(m_done.not());
            let m_prod = m_lo.zext::<64>().mul::<64>(m_d.zext::<64>());
            let m_t =
                m_hi.slice::<0, 32>().concat::<1, 33>(m_lo.slice::<31, 1>());
            let m_fits = lt(m_t, m_d.zext::<33>()).not();
            // An interrupt taken while the sequencer runs cancels it:
            // the instruction starts it again when the handler returns,
            // on the registers as they are then, and the sequencer never
            // steps under an instruction other than its own.
            case!(rst => {
                Bit::One => { self.m_busy <= Bit::Zero },
                _ if int_take.to_bool() => { self.m_busy <= Bit::Zero },
                _ if m_start.to_bool() => {
                    self.m_busy <= Bit::One;
                    self.m_count <= mux(m_is_div, U::from(0u8), U::from(31u8));
                    self.m_hi <= U::from(0u8);
                    self.m_lo <= m_abs_a;
                    self.m_d <= m_abs_b;
                    self.m_neg_q <= mux(
                        m_is_div,
                        m_differ.and(is_zero(b).not()),
                        m_differ
                    );
                    self.m_neg_r <= m_neg_a
                },
                _ if m_step.and(m_is_div.not()).to_bool() => {
                    self.m_count <= m_count.wrapping_add(U::from(1u8));
                    self.m_hi <= m_prod.slice::<32, 32>().zext::<33>();
                    self.m_lo <= m_prod.slice::<0, 32>()
                },
                _ if m_step.to_bool() => {
                    self.m_count <= m_count.wrapping_add(U::from(1u8));
                    self.m_hi <= mux(
                        m_fits,
                        m_t.wrapping_sub(m_d.zext::<33>()),
                        m_t
                    );
                    self.m_lo <=
                        m_lo.slice::<0, 31>().concat::<1, 32>(m_fits.zext())
                },
                _ if run.and(is_m).to_bool() => { self.m_busy <= Bit::Zero },
                _ => {},
            });
            // The writeback stage gets the instruction, or the interrupt
            // in its place, which retires as a trap does: nothing written.
            when!(live => {
                self.wb_valid <= Bit::One;
                self.wb_pc <= pc;
                self.wb_ir <= ir;
                self.wb_rd <= mux(wrote, rd, U::from(0u8));
                self.wb_alu <= wval;
                self.wb_ld <= word;
                self.wb_dev <= dev_word;
                self.wb_is_dev <= is_dev;
                self.wb_f3 <= f3;
                self.wb_lane <= lane;
                self.wb_load <= is_load.and(run);
                self.wb_stop <= is_ebreak.and(run)
            } else {
                self.wb_valid <= Bit::Zero;
                self.wb_rd <= U::from(0u8);
                self.wb_stop <= Bit::Zero
            });
            // begin{fetch}
            // The fetch's next counter: zero on reset, parked on the
            // halting instruction, held while halted or stalled, the
            // target on a redirect, else the next word. Reset, park and
            // hold are known early and the redirect late, so the two
            // candidates fold the early conditions in and the redirect
            // chooses last, one multiplexer from the instruction memory.
            let redirect = run.and(jump).or(int_take);
            let park = run.and(stop);
            let hold = stall.or(stop.and(run.not()));
            let zero = U::<32>::from(0u32);
            let advance =
                mux(hold, fetch_pc, fetch_pc.wrapping_add(U::from(4u8)));
            let go = mux(rst, zero, mux(park, pc, advance));
            let jmp =
                mux(rst, zero, mux(park, pc, mux(int_take, mtvec, target)));
            self.pc.set(mux(redirect, jmp, go));
            case!(rst => {
                Bit::One => { self.valid <= Bit::Zero },
                _ if stall.to_bool() => {},
                _ if stop.to_bool() => { self.valid <= Bit::Zero },
                _ => {
                    self.ir <= fetched;
                    self.ir_pc <= fetch_pc;
                    self.valid <= redirect.not()
                },
            });
            // end{fetch}
            self.stopped.set(stop);
            halt.set(halted);
            instr.set(mux(wb_valid, wb_ir, U::<32>::from(0u32)));
            wb.set(Writeback {
                done: wb_valid,
                rd: wb_rd,
                val: mux(wb_valid, wb_val, U::<32>::from(0u32)),
            });
        }
    }
}
