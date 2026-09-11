// SPDX-License-Identifier: Apache-2.0
//! Run the demonstration program on the core, print a line per cycle,
//! and write the waveform where `TXHDL_FST` points.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{now, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use vreteno32::core::{Vreteno, Writeback};
use vreteno32::isa::disasm;
use vreteno32::program::demo;

fn main() {
    let program = demo();
    let mut cpu = Vreteno::with(&program);
    let (ir_pc, regs, dmem) =
        (cpu.ir_pc.clone(), cpu.regs.clone(), cpu.dmem.clone());
    let (rst_out, rst) = signal::<Bit, DefaultClock>();
    let (halt_out, halt) = signal::<Bit, DefaultClock>();
    let (instr_out, instr) = signal::<U<32>, DefaultClock>();
    let (wb_out, wb) = signal::<Writeback, DefaultClock>();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("rst", &rst);
        w.add("cpu", &cpu);
        w.add("instr", &instr);
        w.add("wb", &wb);
        w.add("halt", &halt);
        w.start();
    }
    let mut sim = Running::new(cpu.run(rst, (halt_out, instr_out, wb_out)));
    rst_out.set(Bit::One);
    sim.cycle();
    rst_out.set(Bit::Zero);
    println!("{:>4} {:>6}  {:<22} {}", "t", "pc", "instruction", "writes");
    for _ in 0..80 {
        let at = ir_pc.get().raw() as u32;
        sim.cycle();
        let w = wb.get();
        if !w.done.to_bool() {
            println!("{:>4} {:>6}  (bubble)", now(), "");
            continue;
        }
        let wrote = if w.rd.raw() != 0 {
            format!("x{} = {:#x}", w.rd.raw(), w.val.raw())
        } else {
            String::new()
        };
        let text = disasm(instr.get().raw() as u32);
        println!("{:>4} {at:#06x}  {text:<22} {wrote}", now());
        if halt.get().to_bool() {
            break;
        }
    }
    stop();
    println!();
    for x in [10, 11, 12, 13, 14, 15, 17, 18, 19, 20] {
        println!("x{x:<2} = {:#010x}", regs.read(x).raw());
    }
    for a in 0..3 {
        println!("mem[{a}] = {:#010x}", dmem.read(a).raw());
    }
}
