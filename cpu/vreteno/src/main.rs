// SPDX-License-Identifier: Apache-2.0
//! Run the demonstration program on the core, print a line per cycle,
//! write the waveform where `TXHDL_FST` points, and the VHDL of the
//! core, with the program in its instruction memory, where
//! `TXHDL_VHDL` points; then print the Verilog.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, join2, now, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::buffer::Buffer;
use vreteno32::core::{Vreteno, Writeback};
use vreteno32::isa::disasm;
use vreteno32::program::demo;
use vreteno32::router::Router;
use vreteno32::term::Terminal;
use vreteno32::timer::Timer;
use vreteno32::uart::Uart;

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
    let (irq_out, irq) = signal::<Bit, DefaultClock>();
    // The bus: a request channel out of the core into the router, a
    // response channel back, and a channel each way from the router to
    // each device, the timer with its interrupt line and the serial
    // port with its two lines and its interrupt, which the core's line
    // carries. The port's bits are four cycles each here, so a byte
    // takes forty.
    let (tirq_out, tirq) = signal::<Bit, DefaultClock>();
    let (tx_out, tx) = signal::<Bit, DefaultClock>();
    let (rx_out, rx) = signal::<Bit, DefaultClock>();
    let (uirq_out, uirq) = signal::<Bit, DefaultClock>();
    let (req_tx, req_rx) = chan::<U<69>, DefaultClock>();
    let (resp_tx, resp_rx) = chan::<U<32>, DefaultClock>();
    let (treq_tx, treq_rx) = chan::<U<69>, DefaultClock>();
    let (tresp_tx, tresp_rx) = chan::<U<32>, DefaultClock>();
    let (ureq_tx, ureq_rx) = chan::<U<69>, DefaultClock>();
    let (uresp_tx, uresp_rx) = chan::<U<32>, DefaultClock>();
    let mut router = Router::default();
    let mut timer = Timer::default();
    let mut uart = Uart::<4>::default();
    let (halt_out, halt) = signal::<Bit, DefaultClock>();
    let (instr_out, instr) = signal::<U<32>, DefaultClock>();
    let (wb_out, wb) = signal::<Writeback, DefaultClock>();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("rst", &rst);
        w.add("irq", &irq);
        w.add("tirq", &tirq);
        w.add("tx", &tx);
        w.add("rx", &rx);
        w.add("uirq", &uirq);
        w.add("resp", &resp_rx);
        w.add("req", &req_rx);
        w.add("treq", &treq_rx);
        w.add("tresp", &tresp_rx);
        w.add("ureq", &ureq_rx);
        w.add("uresp", &uresp_rx);
        w.add("cpu", &cpu);
        w.add("router", &router);
        w.add("timer", &timer);
        w.add("uart", &uart);
        w.add("instr", &instr);
        w.add("wb", &wb);
        w.add("halt", &halt);
        w.start();
    }
    // The timer first: its line is a wire the core reads in the same
    // step, so the process that drives it runs before the one that
    // reads it. The channels between them do not care.
    let (rst_t, rst_u) = (rst.clone(), rst.clone());
    let mut sim = Running::new(join2(
        join2(
            timer.run((rst_t, treq_rx), (tresp_tx, tirq_out)),
            uart.run((rst_u, rx, ureq_rx), (uresp_tx, tx_out, uirq_out)),
        ),
        join2(
            router
                .run((req_rx, tresp_rx, uresp_rx), (treq_tx, ureq_tx, resp_tx)),
            cpu.run(
                (rst, irq, tirq, resp_rx),
                (halt_out, instr_out, wb_out, req_tx),
            ),
        ),
    ));
    rst_out.set(Bit::One);
    sim.cycle();
    rst_out.set(Bit::Zero);
    println!("{:>4} {:>6}  {:<22} {}", "t", "pc", "instruction", "writes");
    // The interrupt line: one pulse, in the loop.
    let irq_at = 40;
    // A run of bubbles prints as one line with its count: a divide is
    // thirty-three of them, a multiply three.
    let mut bubbles: Option<(u64, u32)> = None;
    let flush = |bubbles: &mut Option<(u64, u32)>| {
        if let Some((from, n)) = bubbles.take() {
            if n == 1 {
                println!("{from:>4} {:>6}  (bubble)", "");
            } else {
                println!("{from:>4} {:>6}  ({n} bubbles)", "");
            }
        }
    };
    // The terminal answers the core's line with three bytes, which
    // the program echoes; the port's interrupt joins the line.
    let mut term = Terminal::new(b"yes");
    for cycle in 0..1200 {
        let at = wb_pc.get().raw() as u32;
        irq_out.set(Bit::from_bool(cycle == irq_at).or(uirq.get()));
        rx_out.set(Bit::from_bool(term.level()));
        sim.cycle();
        term.see(tx.get().to_bool());
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
    // The last byte is still going out when the core halts; let the
    // port finish its frame.
    let mut grace = 0;
    while term.busy() && grace < 64 {
        rx_out.set(Bit::from_bool(term.level()));
        sim.cycle();
        term.see(tx.get().to_bool());
        grace += 1;
    }
    flush(&mut bubbles);
    stop();
    println!();
    println!("serial: {:?}", term.said);
    for x in [
        8usize, 10, 11, 12, 13, 14, 15, 17, 18, 19, 20, 23, 24, 25, 26, 29, 30,
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
    // The router, the timer and the serial port, each told under which
    // scope the run traced its channels; the port again at the board's
    // baud rate; and the two channels as hardware, at the widths of the
    // request and the response, for the board to put between them.
    let mut router = Router::lowered("router");
    router.trace_as("timer_req", "treq");
    router.trace_as("timer_resp", "tresp");
    router.trace_as("uart_req", "ureq");
    router.trace_as("uart_resp", "uresp");
    let mut timer = Timer::lowered("timer");
    timer.trace_as("req", "treq");
    timer.trace_as("resp", "tresp");
    let mut uart4 = Uart::<4>::lowered("uart4");
    uart4.trace_as("req", "ureq");
    uart4.trace_as("resp", "uresp");
    uart4.trace_as("irq", "uirq");
    let uart = Uart::<868>::lowered("uart");
    let req_chan = Buffer::<69>::lowered("chan69");
    let resp_chan = Buffer::<32>::lowered("chan32");
    txhdl::netlist::write_netlists_from_env(&[
        &lowered, &router, &timer, &uart4, &uart, &req_chan, &resp_chan,
    ]);
    print!(
        "\n{}\n{}\n{}\n{}",
        lowered.verilog(),
        router.verilog(),
        timer.verilog(),
        uart.verilog()
    );
}
