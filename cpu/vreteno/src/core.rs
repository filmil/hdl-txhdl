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
//! `with!` and `case!` among drives, and nothing branches. The pieces
//! that are functions of their operands alone, the immediates, the
//! ALU, the branch condition, the load's extension, the store's lanes,
//! the CSR access and the sequencer's result, are functions under
//! `#[lower]`, inlined where the step calls them. So the same file
//! simulates and lowers, and the netlist is simulated against the
//! trace the simulation wrote.
use crate::isa;
use txhdl::comp::{
    mux, Clock, DefaultClock, In, Mem, Out, Reg, Rx, Tx, Unit, Wire,
};
use txhdl::funcs::{lt_signed, sra};
use txhdl::types::{Bit, U};
use txhdl::{case, lower, select, when, with, Trace, Value};

/// Words of instruction memory. The data memory is a device on the
/// bus, `crate::dmem`, at `DATA_BASE` as the model has it.
pub const IMEM_WORDS: usize = 1024;
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

// The pieces of the step that are functions of their operands alone,
// each under `#[lower]`, inlined by the lowering where the step calls
// it.

/// The I immediate: the top twelve bits, sign-extended.
#[lower]
fn imm_i(ir: U<32>) -> U<32> {
    ir.slice::<20, 12>().sext::<32>()
}

/// The S immediate, a store's offset, in two pieces.
#[lower]
fn imm_s(ir: U<32>) -> U<32> {
    ir.slice::<25, 7>()
        .concat::<_, 12>(ir.slice::<7, 5>())
        .sext::<32>()
}

/// The B immediate, a branch's offset, in four pieces and even.
#[lower]
fn imm_b(ir: U<32>) -> U<32> {
    ir.slice::<31, 1>()
        .concat::<_, 2>(ir.slice::<7, 1>())
        .concat::<_, 8>(ir.slice::<25, 6>())
        .concat::<_, 12>(ir.slice::<8, 4>())
        .concat::<_, 13>(U::<1>::from(0u8))
        .sext::<32>()
}

/// The U immediate: the top twenty bits, in place.
#[lower]
fn imm_u(ir: U<32>) -> U<32> {
    ir.slice::<12, 20>().concat::<_, 32>(U::<12>::from(0u8))
}

/// The J immediate, a jump's offset, in four pieces and even.
#[lower]
fn imm_j(ir: U<32>) -> U<32> {
    ir.slice::<31, 1>()
        .concat::<_, 9>(ir.slice::<12, 8>())
        .concat::<_, 10>(ir.slice::<20, 1>())
        .concat::<_, 20>(ir.slice::<21, 10>())
        .concat::<_, 21>(U::<1>::from(0u8))
        .sext::<32>()
}

// begin{alu}
/// The ALU: ten operations by `f3`, with `sub` telling subtract from
/// add and the arithmetic shift from the logical.
#[lower]
fn alu(f3: U<3>, sub: Bit, a: U<32>, b: U<32>) -> U<32> {
    let sh = b.slice::<0, 5>();
    select!(f3.raw() => {
        0 => mux(sub, a - b, a + b),
        1 => a << (sh.raw() as usize),
        2 => lt_signed(a, b).zext(),
        3 => Bit::from(a < b).zext(),
        4 => a ^ b,
        5 => mux(sub, sra(a, sh.raw() as usize), a >> (sh.raw() as usize)),
        6 => a | b,
        _ => a & b,
    })
}
// end{alu}

/// Whether a branch is taken, by `f3`.
#[lower]
fn branch(f3: U<3>, a: U<32>, b: U<32>) -> Bit {
    select!(f3.raw() => {
        0 => (a == b).into(),
        1 => (a != b).into(),
        4 => lt_signed(a, b),
        5 => !lt_signed(a, b),
        6 => (a < b).into(),
        _ => (a >= b).into(),
    })
}

/// A loaded word's byte or half, by the lane and the width the load
/// read with, extended; or the word itself.
#[lower]
fn extended(f3: U<3>, lane: U<2>, word: U<32>) -> U<32> {
    let bsh = lane.concat::<_, 5>(U::<3>::from(0u8));
    let hsh = lane.slice::<1, 1>().concat::<_, 5>(U::<4>::from(0u8));
    let octet = (word >> (bsh.raw() as usize)).slice::<0, 8>();
    let half = (word >> (hsh.raw() as usize)).slice::<0, 16>();
    select!(f3.raw() => {
        0 => octet.sext::<32>(),
        1 => half.sext::<32>(),
        2 => word,
        4 => octet.zext::<32>(),
        _ => half.zext::<32>(),
    })
}

/// What each lane takes on a store, the highest lane first: its byte
/// of the word for a word, the half's byte for a half, the byte for a
/// byte.
#[lower]
fn store_data(f3: U<3>, b: U<32>) -> U<32> {
    let b0 = b.slice::<0, 8>();
    let b1 = b.slice::<8, 8>();
    let b2 = b.slice::<16, 8>();
    let b3 = b.slice::<24, 8>();
    let d1 = select!(f3.raw() => { 0 => b0, _ => b1 });
    let d2 = select!(f3.raw() => { 2 => b2, _ => b0 });
    let d3 = select!(f3.raw() => { 0 => b0, 2 => b3, _ => b1 });
    d3.concat::<_, 16>(d2)
        .concat::<_, 24>(d1)
        .concat::<_, 32>(b0)
}

/// Which lanes a store writes, the highest lane first: one for a
/// byte, two for a half, all four for a word.
#[lower]
fn store_lanes(f3: U<3>, lane: U<2>) -> U<4> {
    let upper = lane.bit(1);
    let en0 = select!(f3.raw() => {
        0 => (lane == 0).into(),
        1 => !upper,
        _ => Bit::One,
    });
    let en1 = select!(f3.raw() => {
        0 => (lane == 1).into(),
        1 => !upper,
        _ => Bit::One,
    });
    let en2 = select!(f3.raw() => {
        0 => (lane == 2).into(),
        1 => upper,
        _ => Bit::One,
    });
    let en3 = select!(f3.raw() => {
        0 => (lane == 3).into(),
        1 => upper,
        _ => Bit::One,
    });
    en3.zext::<1>()
        .concat::<_, 2>(en2.zext::<1>())
        .concat::<_, 3>(en1.zext::<1>())
        .concat::<_, 4>(en0.zext::<1>())
}

/// A CSR read, by its number; `mip` is the pending register as the
/// core shows it, with the timer's line in it.
#[lower]
fn csr_read(
    f12: U<12>,
    mstatus: U<32>,
    mtvec: U<32>,
    mscratch: U<32>,
    mepc: U<32>,
    mcause: U<32>,
    mie: U<32>,
    mip: U<32>,
    mtval: U<32>,
) -> U<32> {
    select!(f12.raw() => {
        0x300 => mstatus,
        0x305 => mtvec,
        0x340 => mscratch,
        0x341 => mepc,
        0x342 => mcause,
        0x304 => mie,
        0x344 => mip,
        0x343 => mtval,
        _ => U::<32>::from(0u32),
    })
}

/// Whether a CSR number is one of the eight the core has.
#[lower]
fn csr_known(f12: U<12>) -> Bit {
    select!(f12.raw() => {
        0x300 | 0x305 | 0x340 | 0x341 | 0x342 | 0x304 | 0x344
        | 0x343 => Bit::One,
        _ => Bit::Zero,
    })
}

/// A CSR's new value: replaced, set or cleared by the source, which
/// is a register or a five-bit immediate.
#[lower]
fn csr_value(f3: U<3>, old: U<32>, src: U<32>) -> U<32> {
    select!(f3.raw() => {
        1 | 5 => src,
        2 | 6 => old | src,
        _ => old & !src,
    })
}

/// Whether an M operation takes its first operand as signed: every
/// one but the unsigned multiply-highs, divide and remainder.
#[lower]
fn m_signed_a(f3: U<3>) -> bool {
    (f3 == 0) | (f3 == 1) | (f3 == 2) | (f3 == 4) | (f3 == 6)
}

/// Whether an M operation takes its second operand as signed.
#[lower]
fn m_signed_b(f3: U<3>) -> bool {
    (f3 == 0) | (f3 == 1) | (f3 == 4) | (f3 == 6)
}

/// An M result from the sequencer's registers, with the signs put
/// back: a product or a quotient is negated when the signs differed,
/// a remainder takes the dividend's sign, and a quotient by zero is
/// all ones.
#[lower]
fn m_result(f3: U<3>, hi: U<33>, lo: U<32>, neg_q: Bit, neg_r: Bit) -> U<32> {
    let mag = hi.slice::<0, 32>().concat::<_, 64>(lo);
    let p = mux(neg_q, U::<64>::from(0u32) - mag, mag);
    let q = mux(neg_q, U::<32>::from(0u32) - lo, lo);
    let rem = hi.slice::<0, 32>();
    let r = mux(neg_r, U::<32>::from(0u32) - rem, rem);
    select!(f3.raw() => {
        0 => p.slice::<0, 32>(),
        1 | 2 | 3 => p.slice::<32, 32>(),
        4 | 5 => q,
        _ => r,
    })
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
    pub wb_dev: Reg<U<32>>,
    pub dev_wait: Reg<Bit>,
    /// Three of the cycle's decisions, kept as wires so that a trace
    /// shows them: whether the instruction in execute is stalled, whether
    /// an interrupt takes its place, and whether the fetch is redirected.
    pub stall: Wire<Bit>,
    pub int_take: Wire<Bit>,
    pub redirect: Wire<Bit>,
    pub m_busy: Reg<Bit>,
    pub m_count: Reg<U<6>>,
    pub m_hi: Reg<U<33>>,
    pub m_lo: Reg<U<32>>,
    pub m_d: Reg<U<32>>,
    pub m_neg_q: Reg<Bit>,
    pub m_neg_r: Reg<Bit>,
    pub regs: Mem<U<32>, 32>,
    pub imem: Mem<U<32>, IMEM_WORDS>,
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
impl Unit for Vreteno {
    async fn run(
        &mut self,
        (rst, irq, tirq, resp): (In<Bit>, In<Bit>, In<Bit>, Rx<U<32>>),
        (halt, instr, wb, req): (
            Out<Bit>,
            Out<U<32>>,
            Out<Writeback>,
            Tx<U<69>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let (rst, irq, tirq) = (rst.get(), irq.get(), tirq.get());
            // The registers a step takes apart or hands on as values;
            // the rest are read where they are used.
            let fetch_pc = self.pc.get();
            let (ir, pc) = (self.ir.get(), self.ir_pc.get());
            let (wb_rd, wb_alu) = (self.wb_rd.get(), self.wb_alu.get());
            let (mstatus, mtvec, mepc) =
                (self.mstatus.get(), self.mtvec.get(), self.mepc.get());
            let (mie_r, mip) = (self.mie.get(), self.mip.get());
            // The bus's answer, taken whenever it comes.
            let (resp_valid, resp_data) = resp.take();
            let (m_hi, m_lo, m_d) =
                (self.m_hi.get(), self.m_lo.get(), self.m_d.get());
            // The writeback stage: a loaded word's byte or half, by the
            // lane and the width the execute stage read with it,
            // extended; else the value execute computed. Written to the
            // register file, and forwarded to execute below.
            // A load's word: what the bus answered, in its register.
            let loaded = extended(
                self.wb_f3.get(),
                self.wb_lane.get(),
                self.wb_dev.get(),
            );
            let wb_val = mux(self.wb_load, loaded, wb_alu);
            // A device load sits in writeback while its wait is on, and
            // retires the cycle after its answer has landed.
            let wb_here = self.wb_valid & !self.dev_wait;
            let wb_write = wb_here & (wb_rd != 0);
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
            let imm_i = imm_i(ir);
            let imm_s = imm_s(ir);
            let imm_b = imm_b(ir);
            let imm_u = imm_u(ir);
            let imm_j = imm_j(ir);
            // The operands: register zero reads as zero, and a register
            // the writeback stage is about to write reads as the value it
            // will write, which is the forwarding path. A load is the
            // exception: its word lands at the edge and is extended in
            // writeback, too long a path to forward through the ALU into
            // the fetch, so an instruction that reads what the load in
            // writeback will write waits one cycle and reads the register
            // file, which has it by then. The stall is the one cycle the
            // core ever waits.
            let fwd_a = wb_write & (wb_rd == rs1);
            let fwd_b = wb_write & (wb_rd == rs2);
            let stall_ld = self.valid & self.wb_load & (fwd_a | fwd_b);
            // The M extension is a sequencer: a multiply is one step in
            // the part's multipliers, a division thirty-two restoring
            // steps, over the magnitudes, and the instruction stalls in
            // execute until the count is up. One set of registers serves
            // both.
            let f7 = ir.slice::<25, 7>();
            let is_m = (opcode == 0x33) & (f7 == 1);
            let m_here = self.valid & !rst & !self.stopped & is_m;
            let m_done = self.m_busy & (self.m_count == 32);
            // An interrupt is taken instead of the instruction in execute
            // when it is pending, enabled, and interrupts are enabled; an
            // M instruction under one does not start, so it cannot hold
            // the stall the interrupt waits for.
            // The timer's line is a register's output on the timer's
            // side, so it is read as state is.
            let mtip = tirq;
            let ext_ok = mie_r.bit(11) & mip.bit(11);
            let tim_ok = mie_r.bit(7) & mtip;
            let int_ok = mstatus.bit(3) & (ext_ok | tim_ok);
            let stall_m = m_here & !int_ok & !m_done;
            // A load or store to the bus, which is everything above the
            // data memory. A store goes out when the bus has room; a
            // load goes out and moves on to writeback, which holds it
            // until the answer has landed in its register there.
            let a = mux(
                rs1 == 0,
                U::<32>::from(0u32),
                mux(fwd_a, wb_alu, self.regs.read(rs1)),
            );
            let b = mux(
                rs2 == 0,
                U::<32>::from(0u32),
                mux(fwd_b, wb_alu, self.regs.read(rs2)),
            );
            let addr = a + select!(opcode.raw() => {
                0x23 => imm_s,
                _ => imm_i,
            });
            let is_load = opcode == 0x03;
            let is_store = opcode == 0x23;
            // Everything from the data memory up is on the bus.
            let is_dev = addr.slice::<12, 20>() != 0;
            let here = self.valid & !rst & !self.stopped;
            // A load or a store waits for room on the bus whatever its
            // address, so that the fetch's hold does not hang on the
            // address's decode, which was the path that limited the
            // clock; there is room whenever the devices keep up, and
            // they do. A device load's wait for its answer is a
            // register, so the hold for it is state too.
            let stall_bus = here & (is_load | is_store) & !req.ready();
            self.stall
                .set(stall_ld | stall_m | stall_bus | self.dev_wait);
            let stall = self.stall.get();
            let pc4 = pc + 4;
            // Live: an instruction in execute that is not stalled. It
            // runs unless the interrupt takes its place.
            let live = !rst & !self.stopped & self.valid & !stall;
            self.int_take.set(live & int_ok);
            let int_take = self.int_take.get();
            let run = live & !int_take;
            let send_load = run & is_load & is_dev;

            // The ALU, shared by the register and immediate forms; bit
            // 30 means subtract or arithmetic shift, except that an
            // immediate may have it set and mean nothing by it.
            let alu_b = mux(opcode == 0x13, imm_i, b);
            let sub = alt & ((opcode == 0x33) | (f3 == 5));
            // The multiply and divide, from the sequencer's registers.
            let m_res = m_result(
                f3,
                m_hi,
                m_lo,
                self.m_neg_q.get(),
                self.m_neg_r.get(),
            );
            let alu = alu(f3, sub, a, alu_b);
            // The branch condition.
            let taken = branch(f3, a, b);
            // Loads and stores go out on the bus: the lane within the
            // word, what each lane takes on a store, and which lanes.
            let lane = addr.slice::<0, 2>();
            // What each lane takes on a store, and which lanes take it.
            let sdata = store_data(f3, b);
            let en = store_lanes(f3, lane);

            // What the instruction does: the value it writes back, if
            // any, where it goes next, and whether the core knows it.
            let writes = select!(opcode.raw() => {
                0x37 | 0x17 | 0x6f | 0x67 | 0x03 | 0x13 | 0x33 => Bit::One,
                0x73 => (f3 != 0).into(),
                _ => Bit::Zero,
            });
            // The system instructions. A CSR instruction reads one of
            // the five registers and writes it, set or cleared or
            // replaced, from a register or a five-bit immediate; ecall
            // and an instruction the core does not know trap; mret
            // returns; ebreak halts.
            let f12 = ir.slice::<20, 12>();
            let is_sys = opcode == 0x73;
            let csr_op = is_sys & (f3 != 0);
            let mip_now = mux(tirq, mip | isa::MTIMER, mip);
            let csr_old = csr_read(
                f12,
                mstatus,
                mtvec,
                self.mscratch.get(),
                mepc,
                self.mcause.get(),
                mie_r,
                mip_now,
                self.mtval.get(),
            );
            let csr_known = csr_known(f12);
            let csr_src = mux(f3.bit(2), rs1.zext::<32>(), a);
            let csr_new = csr_value(f3, csr_old, csr_src);
            let sys0 = is_sys & (f3 == 0);
            let is_ecall = sys0 & (f12 == 0);
            let is_ebreak = sys0 & (f12 == 1);
            let is_mret = sys0 & (f12 == 0x302);
            let known = select!(opcode.raw() => {
                0x37 | 0x17 | 0x6f | 0x67 | 0x63 | 0x03 | 0x23 | 0x13 | 0x33
                | 0x0f => Bit::One,
                0x73 => (csr_op & csr_known) | is_ecall | is_ebreak | is_mret,
                _ => Bit::Zero,
            });
            // A trap: ecall, a word the core does not know, or the
            // interrupt. The cause, the address and the trap value go to
            // the CSRs, the interrupt enable is saved and cleared, and
            // the handler is the redirect.
            let trap = (run & (is_ecall | !known)) | int_take;
            let cause = mux(
                int_take,
                mux(
                    ext_ok,
                    U::<32>::from(isa::CAUSE_MEXT),
                    U::<32>::from(isa::CAUSE_MTIMER),
                ),
                mux(
                    is_ecall,
                    U::<32>::from(isa::CAUSE_ECALL),
                    U::<32>::from(isa::CAUSE_ILLEGAL),
                ),
            );
            let tval = mux(run & !known, ir, U::<32>::from(0u32));
            let mie_bit = mstatus.bit(3);
            let mpie = mstatus.bit(7);
            let trap_status =
                mux(mie_bit, U::<32>::from(0x80u32), U::<32>::from(0u32));
            let mret_status =
                mux(mpie, U::<32>::from(0x88u32), U::<32>::from(0x80u32));
            let csr_write = run & csr_op & csr_known;
            // Stopped: on ebreak, and then for good; the halt itself
            // follows a cycle later, when the halting instruction
            // retires.
            let stop = mux(run, Bit::from(is_ebreak), self.stopped.get());
            let wrote = run & writes & (rd != 0) & !trap;
            let store = run & is_store;
            let wval = select!(opcode.raw() => {
                0x37 => imm_u,
                0x17 => pc + imm_u,
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
                0x67 => (a + imm_i) & !U::<32>::from(1u32),
                0x6f => pc + imm_j,
                0x63 => pc + imm_b,
                0x73 => mux(is_mret, mepc, mtvec),
                _ => mtvec,
            });
            let jump = select!(opcode.raw() => {
                0x6f | 0x67 => Bit::One,
                0x63 => taken,
                0x73 => is_mret | trap,
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
            when!(wb_write => self { regs.at(wb_rd): wb_val });
            self.halted.set(self.halted | self.wb_stop);
            // The bus: a request is the address, the data in its lanes,
            // the lanes a store covers, and whether it is a store. The
            // load's wait is a register.
            let req_word = addr
                .concat::<_, 64>(sdata)
                .concat::<_, 68>(en)
                .concat::<_, 69>(store.zext::<1>());
            if bool::from(send_load | (store & is_dev)) {
                req.send(req_word);
            }
            when!(resp_valid => self { wb_dev: resp_data });
            case!(rst => {
                Bit::One => { self.dev_wait <= Bit::Zero },
                _ if send_load.to_bool() => { self.dev_wait <= Bit::One },
                _ if (self.dev_wait & resp_valid).to_bool() => {
                    self.dev_wait <= Bit::Zero
                },
                _ => {},
            });

            // The CSRs: written by a CSR instruction, by a trap, by mret.
            // The three never coincide in one instruction.
            // The pending bit is set by the line and cleared by
            // software, and the write wins when both fall in one cycle;
            // a trap's writes come after the CSR writes, and win.
            with!(self <= {
                csr_write & (f12 == isa::CSR_MSTATUS) ?
                    mstatus: csr_new & 0x88,
                csr_write & (f12 == isa::CSR_MTVEC) ?
                    mtvec: csr_new & !U::<32>::from(3u32),
                csr_write & (f12 == isa::CSR_MSCRATCH) ? mscratch: csr_new,
                csr_write & (f12 == isa::CSR_MEPC) ?
                    mepc: csr_new & !U::<32>::from(1u32),
                csr_write & (f12 == isa::CSR_MCAUSE) ? mcause: csr_new,
                csr_write & (f12 == isa::CSR_MIE) ?
                    mie: csr_new & U::<32>::from(isa::MEXT | isa::MTIMER),
                csr_write & (f12 == isa::CSR_MTVAL) ? mtval: csr_new,
                irq ? mip: mip | isa::MEXT,
                csr_write & (f12 == isa::CSR_MIP) ? mip: csr_new & isa::MEXT,
                trap ? {
                    mepc: pc,
                    mcause: cause,
                    mtval: tval,
                    mstatus: trap_status,
                },
                run & is_mret ? mstatus: mret_status,
            });
            // The sequencer. It starts when an M instruction is in execute
            // with its operands ready, and is released when the
            // instruction runs. A multiply is one step: the product of
            // the magnitudes, from the registers into the pair, which the
            // part's DSP blocks make; a division is thirty-two, each
            // shifting the pair left, subtracting the divisor from the
            // high half when it fits, and shifting the fit in as the
            // quotient bit.
            let m_signed_a = m_signed_a(f3);
            let m_signed_b = m_signed_b(f3);
            let m_neg_a = m_signed_a & a.bit(31);
            let m_neg_b = m_signed_b & b.bit(31);
            let m_abs_a = mux(m_neg_a, U::<32>::from(0u32) - a, a);
            let m_abs_b = mux(m_neg_b, U::<32>::from(0u32) - b, b);
            let m_differ = (m_neg_a & !m_neg_b) | (!m_neg_a & m_neg_b);
            let m_is_div = f3.bit(2);
            let m_start = m_here & !self.m_busy & !stall_ld & !int_ok;
            let m_step = self.m_busy & !m_done;
            let m_prod = m_lo.zext::<64>().mul::<64>(m_d.zext::<64>());
            let m_t =
                m_hi.slice::<0, 32>().concat::<_, 33>(m_lo.slice::<31, 1>());
            let m_fits = m_t >= m_d.zext::<33>();
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
                    self.m_hi <= 0;
                    self.m_lo <= m_abs_a;
                    self.m_d <= m_abs_b;
                    self.m_neg_q <= mux(
                        m_is_div,
                        m_differ & (b != 0),
                        m_differ
                    );
                    self.m_neg_r <= m_neg_a
                },
                _ if (m_step & !m_is_div).to_bool() => {
                    self.m_count <= self.m_count + 1;
                    self.m_hi <= m_prod.slice::<32, 32>().zext::<33>();
                    self.m_lo <= m_prod.slice::<0, 32>()
                },
                _ if m_step.to_bool() => {
                    self.m_count <= self.m_count + 1;
                    self.m_hi <= mux(m_fits, m_t - m_d.zext::<33>(), m_t);
                    self.m_lo <= m_lo
                        .slice::<0, 31>()
                        .concat::<1, 32>(Bit::from(m_fits).zext())
                },
                _ if (run & is_m).to_bool() => { self.m_busy <= Bit::Zero },
                _ => {},
            });
            // The writeback stage gets the instruction, or the interrupt
            // in its place, which retires as a trap does: nothing written.
            case!(live => {
                Bit::One => {
                    self.wb_valid <= Bit::One;
                    self.wb_pc <= pc;
                    self.wb_ir <= ir;
                    self.wb_rd <= mux(wrote, rd, U::from(0u8));
                    self.wb_alu <= wval;
                    self.wb_f3 <= f3;
                    self.wb_lane <= lane;
                    self.wb_load <= is_load & run;
                    self.wb_stop <= is_ebreak & run
                },
                _ if self.dev_wait.to_bool() => {},
                _ => {
                    self.wb_valid <= Bit::Zero;
                    self.wb_rd <= 0;
                    self.wb_stop <= Bit::Zero
                },
            });
            // begin{fetch}
            // The fetch's next counter: zero on reset, parked on the
            // halting instruction, held while halted or stalled, the
            // target on a redirect, else the next word. Reset, park and
            // hold are known early and the redirect late, so the two
            // candidates fold the early conditions in and the redirect
            // chooses last, one multiplexer from the instruction memory.
            self.redirect.set((run & jump) | int_take);
            let redirect = self.redirect.get();
            let park = run & stop;
            let hold = stall | (stop & !run);
            let zero = U::<32>::from(0u32);
            let advance = mux(hold, fetch_pc, fetch_pc + 4);
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
                    self.valid <= !redirect
                },
            });
            // end{fetch}
            self.stopped.set(stop);
            halt.set(self.halted);
            instr.set(mux(wb_here, self.wb_ir.get(), U::<32>::from(0u32)));
            wb.set(Writeback {
                done: wb_here,
                rd: wb_rd,
                val: mux(wb_here, wb_val, U::<32>::from(0u32)),
            });
        }
    }
}
