// SPDX-License-Identifier: Apache-2.0
//! The core against the model, every cycle: the program counter, the
//! thirty-one registers and the halt, and the data memory at the end.
//! The demonstration program and a batch of random ones.
use txhdl::comp::{signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use vreteno32::core::{Vreteno, Writeback};
use vreteno32::isa::disasm;
use vreteno32::model::{Halt, Model};
use vreteno32::program::{demo, random};

/// Runs `program` on both until the core halts, checking after every
/// cycle; returns the model at the halt.
fn lockstep(program: &[u32], what: &str) -> Model {
    let mut cpu = Vreteno::with(program);
    let (pc, regs, dmem, halted) = (
        cpu.pc.clone(),
        cpu.regs.clone(),
        cpu.dmem.clone(),
        cpu.halted.clone(),
    );
    let (rst_out, rst) = signal::<Bit, DefaultClock>();
    let (halt_out, _halt) = signal::<Bit, DefaultClock>();
    let (instr_out, instr) = signal::<U<32>, DefaultClock>();
    let (wb_out, _wb) = signal::<Writeback, DefaultClock>();
    let mut sim = Running::new(cpu.run(rst, (halt_out, instr_out, wb_out)));
    rst_out.set(Bit::One);
    sim.cycle();
    rst_out.set(Bit::Zero);
    let mut model = Model::default();
    for cycle in 0..4096 {
        let at = model.pc;
        sim.cycle();
        model.step(program);
        let here = format!(
            "{what}, cycle {cycle}, pc {at:#x}: {}",
            disasm(instr.get().raw() as u32)
        );
        assert_eq!(pc.get().raw() as u32, model.pc, "pc after {here}");
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
        if model.halted.is_some() {
            for (a, &w) in model.mem.iter().enumerate() {
                assert_eq!(dmem.read(a).raw() as u32, w, "mem[{a}] {here}");
            }
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
}

#[test]
fn random_programs() {
    for seed in 0..64 {
        let p = random(seed, 200);
        let m = lockstep(&p, &format!("random seed {seed}"));
        assert_eq!(m.halted, Some(Halt::Break), "seed {seed} faulted");
    }
}
