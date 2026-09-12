// SPDX-License-Identifier: Apache-2.0
//! The core against the model, every cycle: the model steps when the
//! core retires an instruction, and then the program counter, the
//! thirty-one registers, the control registers and the halt must
//! agree, and the data memory at the end. The demonstration program
//! and a batch of random ones.
use txhdl::comp::{signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use vreteno32::core::{Vreteno, Writeback};
use vreteno32::isa::{decode, disasm, Kind};
use vreteno32::model::{Halt, Model};
use vreteno32::program::{demo, random};

/// Runs `program` on both until the core halts, checking after every
/// cycle; returns the model at the halt.
fn lockstep(program: &[u32], what: &str) -> Model {
    let mut cpu = Vreteno::with(program);
    let (pc, ir_pc, valid, regs, halted) = (
        cpu.pc.clone(),
        cpu.ir_pc.clone(),
        cpu.valid.clone(),
        cpu.regs.clone(),
        cpu.halted.clone(),
    );
    let lanes = (
        cpu.dmem0.clone(),
        cpu.dmem1.clone(),
        cpu.dmem2.clone(),
        cpu.dmem3.clone(),
    );
    let word = move |a: usize| -> u32 {
        (lanes.0.read(a).raw() as u32)
            | (lanes.1.read(a).raw() as u32) << 8
            | (lanes.2.read(a).raw() as u32) << 16
            | (lanes.3.read(a).raw() as u32) << 24
    };
    let (wb_valid, wb_pc) = (cpu.wb_valid.clone(), cpu.wb_pc.clone());
    let csrs = [
        ("mstatus", cpu.mstatus.clone()),
        ("mtvec", cpu.mtvec.clone()),
        ("mscratch", cpu.mscratch.clone()),
        ("mepc", cpu.mepc.clone()),
        ("mcause", cpu.mcause.clone()),
    ];
    // The architectural program counter, as Vreteno::arch_pc has it:
    // the oldest instruction not yet retired.
    let arch_pc = move || {
        if wb_valid.get().to_bool() {
            wb_pc.get()
        } else if valid.get().to_bool() {
            ir_pc.get()
        } else {
            pc.get()
        }
    };
    let (rst_out, rst) = signal::<Bit, DefaultClock>();
    let (halt_out, _halt) = signal::<Bit, DefaultClock>();
    let (instr_out, _instr) = signal::<U<32>, DefaultClock>();
    let (ir, in_execute) = (cpu.ir.clone(), cpu.valid.clone());
    let (wb_out, wb) = signal::<Writeback, DefaultClock>();
    let mut sim = Running::new(cpu.run(rst, (halt_out, instr_out, wb_out)));
    rst_out.set(Bit::One);
    sim.cycle();
    rst_out.set(Bit::Zero);
    let mut model = Model::default();
    let mut retired = 0;
    for cycle in 0..4096 {
        let at = model.pc;
        // The word about to execute this cycle, or zero on a bubble.
        let executed = if in_execute.get().to_bool() {
            ir.get().raw() as u32
        } else {
            0
        };
        sim.cycle();
        if wb.get().done.to_bool() {
            model.step(program);
            retired += 1;
        }
        let here =
            format!("{what}, cycle {cycle}, pc {at:#x}: {}", disasm(executed));
        assert_eq!(arch_pc().raw() as u32, model.pc, "pc after {here}");
        for x in 1..32 {
            assert_eq!(
                regs.read(x).raw() as u32,
                model.x[x],
                "x{x} after {here}"
            );
        }
        assert_eq!(
            halted.get().to_bool(),
            model.halted.is_some(),
            "halt after {here}"
        );
        // The core writes a CSR in execute, a stage before the
        // instruction retires, so that the next instruction sees it;
        // the model writes it when the instruction retires. So the
        // CSRs are compared except in the cycle a system instruction,
        // or an illegal word, executed: they agree again a cycle later.
        let system = matches!(
            decode(executed).kind,
            Kind::Csrrw
                | Kind::Csrrs
                | Kind::Csrrc
                | Kind::Csrrwi
                | Kind::Csrrsi
                | Kind::Csrrci
                | Kind::Ecall
                | Kind::Mret
                | Kind::Illegal
        );
        let want = [
            model.csr.mstatus,
            model.csr.mtvec,
            model.csr.mscratch,
            model.csr.mepc,
            model.csr.mcause,
        ];
        for ((name, r), w) in csrs.iter().zip(want) {
            if !system {
                assert_eq!(r.get().raw() as u32, w, "{name} after {here}");
            }
        }
        if model.halted.is_some() {
            for (a, &w) in model.mem.iter().enumerate() {
                assert_eq!(word(a), w, "mem[{a}] {here}");
            }
            // A pipeline retires at most one per cycle; the difference
            // is the bubbles, one per taken branch and jump.
            assert!(retired <= cycle + 1, "{what}: retired {retired}");
            return model;
        }
    }
    panic!("{what}: no halt in 4096 cycles");
}

#[test]
fn demo_program() {
    let m = lockstep(&demo(), "demo");
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[10], 110);
    assert_eq!(m.mem[0], 110);
    assert_eq!(m.x[11], -2i32 as u32);
    assert_eq!(m.x[12], 254);
    assert_eq!(m.x[13], -2i32 as u32);
    assert_eq!(m.x[14], 65534);
    assert_eq!(m.x[23], 2, "the second trap's cause");
    assert_eq!(m.x[24], 5, "mscratch through the CSR instructions");
    assert_eq!(m.x[25], 0xfe01, "the use right after the load");
    assert_eq!(m.x[26], (-220i32) as u32, "mul");
    assert_eq!(m.x[28], 0xfffffffc, "mulhu");
    assert_eq!(m.x[29], (-55i32) as u32, "div");
    assert_eq!(m.x[30], (-2i32) as u32, "rem");
}

#[test]
fn random_programs() {
    for seed in 0..64 {
        let p = random(seed, 200);
        let m = lockstep(&p, &format!("random seed {seed}"));
        assert_eq!(m.halted, Some(Halt::Break), "seed {seed} faulted");
    }
}
