// SPDX-License-Identifier: Apache-2.0
//! Run the demonstration program on the core, print a line per cycle,
//! write the waveform where `TXHDL_FST` points, and the VHDL of the
//! core, with the program in its instruction memory, where
//! `TXHDL_VHDL` points; then print the Verilog.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{now, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use vreteno32::core::{Vreteno, Writeback};
use vreteno32::isa::disasm;
use vreteno32::program::demo;

fn main() {
    let program = demo();
    let mut cpu = Vreteno::with(&program);
    let (wb_pc, regs) = (cpu.wb_pc.clone(), cpu.regs.clone());
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
    // A run of bubbles prints as one line with its count: a multiply
    // or a divide is thirty-three of them.
    let mut bubbles: Option<(u64, u32)> = None;
    let mut flush = |bubbles: &mut Option<(u64, u32)>| {
        if let Some((from, n)) = bubbles.take() {
            if n == 1 {
                println!("{from:>4} {:>6}  (bubble)", "");
            } else {
                println!("{from:>4} {:>6}  ({n} bubbles)", "");
            }
        }
    };
    for _ in 0..400 {
        let at = wb_pc.get().raw() as u32;
        sim.cycle();
        let w = wb.get();
        if !w.done.to_bool() {
            bubbles = match bubbles {
                Some((from, n)) => Some((from, n + 1)),
                None => Some((now(), 1)),
            };
            // The halt follows the halting instruction by a cycle, a
            // bubble.
            if halt.get().to_bool() {
                break;
            }
            continue;
        }
        flush(&mut bubbles);
        let wrote = if w.rd.raw() != 0 {
            format!("x{} = {:#x}", w.rd.raw(), w.val.raw())
        } else {
            String::new()
        };
        let text = disasm(instr.get().raw() as u32);
        println!("{:>4} {at:#06x}  {text:<22} {wrote}", now());
    }
    flush(&mut bubbles);
    stop();
    println!();
    for x in [
        10usize, 11, 12, 13, 14, 15, 17, 18, 19, 20, 23, 24, 25, 26, 29, 30,
    ] {
        println!("x{x:<2} = {:#010x}", regs.read(x).raw());
    }
    for a in 0..3usize {
        println!("mem[{a}] = {:#010x}", word(a));
    }
    // The netlist, with the program in its instruction memory, which
    // the lowering cannot see: Mem::with gave it at run time.
    let mut lowered = Vreteno::lowered("vreteno");
    let words: Vec<u128> = program.iter().map(|&w| w as u128).collect();
    lowered.init("imem", &words);
    txhdl::netlist::write_vhdl_from_env(&lowered);
    print!("\n{}", lowered.verilog());
}
