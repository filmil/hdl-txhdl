// SPDX-License-Identifier: Apache-2.0
//! The core against the model, every cycle: the model steps when the
//! core retires an instruction, and then the program counter, the
//! thirty-one registers, the control registers and the halt must
//! agree, and the data memory at the end. The demonstration program
//! and a batch of random ones.
use txhdl::comp::{join2, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi_units, AxiHost, AxiPer};
use txhdl_parts::bus::router::Router3;
use vreteno32::core::{Vreteno, Writeback};
use vreteno32::dmem::Dmem;
use vreteno32::isa::{decode, disasm, Kind, CAUSE_MEXT, CAUSE_MTIMER};
use vreteno32::model::{Halt, Model};
use vreteno32::program::{demo, random};
use vreteno32::term::Terminal;
use vreteno32::timer::Timer;
use vreteno32::uart::Uart;

/// The link the core sits on, as the demonstration has it.
const IW: usize = 2;
const NIDS: usize = 4;
type Rtr =
    Router3<32, 32, 4, IW, 0x1000, 0xf000, 0x2000, 0xf000, 0x3000, 0xf000>;

/// Runs `program` on both until the core halts, checking after every
/// cycle; returns the model at the halt.
fn lockstep(program: &[u32], what: &str, seed: Option<u64>) -> Model {
    let mut cpu = Vreteno::with(program);
    let (pc, ir_pc, valid, regs, halted) = (
        cpu.pc.clone(),
        cpu.ir_pc.clone(),
        cpu.valid.clone(),
        cpu.regs.clone(),
        cpu.halted.clone(),
    );
    let mut dmem = Dmem::<IW>::default();
    let lanes = (
        dmem.lane0.clone(),
        dmem.lane1.clone(),
        dmem.lane2.clone(),
        dmem.lane3.clone(),
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
        ("mie", cpu.mie.clone()),
        ("mip", cpu.mip.clone()),
        ("mtval", cpu.mtval.clone()),
    ];
    let (mip, mie, mstatus) =
        (cpu.mip.clone(), cpu.mie.clone(), cpu.mstatus.clone());

    let mut timer = Timer::<IW>::default();
    let mut uart = Uart::<4, IW>::default();
    let mtimecmp = timer.mtimecmp.clone();
    let (pending, wb_dev) = (timer.pending.clone(), cpu.wb_dev.clone());
    let (uart_sent, uart_last) = (uart.sent.clone(), uart.last.clone());
    let (uart_received, uart_dropped) =
        (uart.received.clone(), uart.dropped.clone());
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
    let (irq_out, irq) = signal::<Bit, DefaultClock>();
    let (tirq_out, tirq) = signal::<Bit, DefaultClock>();
    let (tx_out, tx) = signal::<Bit, DefaultClock>();
    let (rx_out, rx) = signal::<Bit, DefaultClock>();
    let (uirq_out, uirq) = signal::<Bit, DefaultClock>();
    // The core's link, and one per peripheral, with the router
    // between the core's tracker and the three peripherals'.
    let cl = axi_units::<32, 32, 4, IW>();
    let dl = axi_units::<32, 32, 4, IW>();
    let tl = axi_units::<32, 32, 4, IW>();
    let ul = axi_units::<32, 32, 4, IW>();
    let (issue, wbeat, release, grant, cdone, crdata) = cl.host_client;
    let (dreq, dwd, dans, drb) = dl.per_client;
    let (treq, twd, tans, trb) = tl.per_client;
    let (ureq, uwd, uans, urb) = ul.per_client;
    let mut axi_host = AxiHost::<32, 32, 4, IW, NIDS>::default();
    let mut dper = AxiPer::<32, 32, 4, IW>::default();
    let mut tper = AxiPer::<32, 32, 4, IW>::default();
    let mut uper = AxiPer::<32, 32, 4, IW>::default();
    let mut router = Rtr::default();
    let (halt_out, _halt) = signal::<Bit, DefaultClock>();
    let (instr_out, _instr) = signal::<U<32>, DefaultClock>();
    let (ir, in_execute) = (cpu.ir.clone(), cpu.valid.clone());
    let stall = cpu.stall.clone();
    let (wb_out, wb) = signal::<Writeback, DefaultClock>();
    // The timer first, since the core reads its line in the same step.
    let (rst_t, rst_u) = (rst.clone(), rst.clone());
    let mut sim = Running::new(join2(
        join2(
            join2(
                timer.run((rst_t, treq, twd), (tans, trb, tirq_out)),
                uart.run((rst_u, rx, ureq, uwd), (uans, urb, tx_out, uirq_out)),
            ),
            join2(
                dmem.run((dreq, dwd), (dans, drb)),
                cpu.run(
                    (rst, irq, tirq, crdata, cdone, grant),
                    (halt_out, instr_out, wb_out, issue, wbeat, release),
                ),
            ),
        ),
        join2(
            join2(
                axi_host.run(cl.host_in, cl.host_out),
                router.run(
                    (
                        cl.per_in.0,
                        cl.per_in.1,
                        cl.per_in.2,
                        dl.host_in.2,
                        dl.host_in.3,
                        tl.host_in.2,
                        tl.host_in.3,
                        ul.host_in.2,
                        ul.host_in.3,
                    ),
                    (
                        dl.host_out.0,
                        dl.host_out.1,
                        dl.host_out.2,
                        tl.host_out.0,
                        tl.host_out.1,
                        tl.host_out.2,
                        ul.host_out.0,
                        ul.host_out.1,
                        ul.host_out.2,
                        cl.per_out.2,
                        cl.per_out.3,
                    ),
                ),
            ),
            join2(
                join2(
                    dper.run(dl.per_in, dl.per_out),
                    tper.run(tl.per_in, tl.per_out),
                ),
                uper.run(ul.per_in, ul.per_out),
            ),
        ),
    ));
    rst_out.set(Bit::One);
    sim.cycle();
    rst_out.set(Bit::Zero);
    let mut model = Model::default();
    let mut retired = 0;
    // The interrupt line, from the seed: high now and then for the
    // random programs, one pulse in the loop for the demonstration.
    let mut noise = seed.unwrap_or(0).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    // Whether the core, deciding on its registers as they stand before
    // this cycle, takes the interrupt in place of the instruction in
    // execute; the model is told so when that instruction retires. The
    // decision is the one of the instruction's last cycle in execute,
    // the cycle it is not stalled, which the core's stall wire says
    // once the cycle has run: an instruction may sit stalled behind a
    // device load's wait for cycles in which the registers move on.
    let mut taken: Option<u32> = None;
    let mut taken_before: Option<u32> = None;
    // What the bus answered a load from a device is in the core's own
    // register for it when the load retires, and is handed to the model
    // with the instruction, since the model has no bus; the timer's line
    // as the timer registered it goes with it.
    let mut line = false;
    let mut line_before = false;
    let mut answer = 0u32;
    // The terminal on the port's lines: it answers the demonstration's
    // line with three bytes, which the program echoes; a random program
    // gets nothing typed. The port's interrupt joins the core's line,
    // as it does on the board.
    let mut term = Terminal::new(if seed.is_none() { b"yes" } else { b"" });
    for cycle in 0..32768 {
        let at = model.pc;
        let pulse = match seed {
            None => cycle == 40,
            Some(_) => {
                noise ^= noise << 13;
                noise ^= noise >> 7;
                noise ^= noise << 17;
                noise & 15 == 0
            }
        };
        let raised = pulse || uirq.get().to_bool();
        irq_out.set(raised);
        rx_out.set(term.level());
        // The word about to execute this cycle, or zero on a bubble;
        // the illegal word is zero too, so the flag is kept apart.
        let executing = in_execute.get().to_bool();
        let executed = if executing { ir.get().raw() as u32 } else { 0 };
        // The register holds the load's answer until the cycle in
        // which it retires, which is the next instruction's execute
        // cycle, so it is read before that cycle.
        answer = wb_dev.get().raw() as u32;
        let line_now = pending.get().to_bool();
        let ext = mip.get().bit(11).to_bool() && mie.get().bit(11).to_bool();
        let tim = line_now && mie.get().bit(7).to_bool();
        let taken_now = if !mstatus.get().bit(3).to_bool() {
            None
        } else if ext {
            Some(CAUSE_MEXT)
        } else if tim {
            Some(CAUSE_MTIMER)
        } else {
            None
        };
        sim.cycle();
        term.see(tx.get().to_bool());
        if executing && !stall.get().to_bool() {
            line = line_now;
            taken = taken_now;
        }
        if wb.get().done.to_bool() {
            model.dev_word = answer;
            model.tirq = line_before;
            model.step(program, taken_before);
            retired += 1;
        }
        taken_before = taken;
        line_before = line;
        // The line sets the pending bit at this edge in both.
        if raised {
            model.raise();
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
        let system = executing
            && !stall.get().to_bool()
            && (taken.is_some()
                || matches!(
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
                        | Kind::Sb
                        | Kind::Sh
                        | Kind::Sw
                ));
        let want = [
            model.csr.mstatus,
            model.csr.mtvec,
            model.csr.mscratch,
            model.csr.mepc,
            model.csr.mcause,
            model.csr.mie,
            model.csr.mip,
            model.csr.mtval,
        ];
        for ((name, r), w) in csrs.iter().zip(want) {
            if !system {
                assert_eq!(r.get().raw() as u32, w, "{name} after {here}");
            }
        }
        // The compare lands in the timer some cycles after the core's
        // store, since a store is posted and the bus carries it, so it
        // is checked at the end, as the memory is, and the bus is let
        // drain first.
        if model.halted.is_some() {
            for _ in 0..32 {
                sim.cycle();
            }
            assert_eq!(
                mtimecmp.get().raw() as u64,
                model.mtimecmp,
                "mtimecmp at the halt, {here}"
            );
            // The serial port took every byte the model has, in order;
            // the port keeps the count and the last.
            assert_eq!(
                uart_sent.get().raw() as usize,
                model.uart.len(),
                "bytes to the serial port, {here}"
            );
            if let Some(&last) = model.uart.last() {
                assert_eq!(uart_last.get().raw() as u8, last, "last byte");
            }
            // The terminal's bytes all came in and none found the
            // buffer full; the demonstration echoed every one.
            assert_eq!(uart_dropped.get().raw(), 0, "bytes dropped, {here}");
            if seed.is_none() {
                assert_eq!(uart_received.get().raw(), 3, "bytes received");
            }
            for (a, &w) in model.mem.iter().enumerate() {
                assert_eq!(word(a), w, "mem[{a}] {here}");
            }
            // A pipeline retires at most one per cycle; the difference
            // is the bubbles, one per taken branch and jump.
            assert!(retired <= cycle + 1, "{what}: retired {retired}");
            return model;
        }
    }
    panic!("{what}: no halt in 32768 cycles");
}

#[test]
fn demo_program() {
    let m = lockstep(&demo(), "demo", None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[10], 110);
    assert_eq!(m.mem[0], 110);
    assert_eq!(m.x[11], -2i32 as u32);
    assert_eq!(m.x[12], 254);
    assert_eq!(m.x[13], -2i32 as u32);
    assert_eq!(m.x[14], 65534);
    assert_eq!(m.x[23], 2, "the second trap's cause");
    assert_eq!(m.x[24], 5, "mscratch through the CSR instructions");
    assert_eq!(m.x[8], 2, "the line's and the timer's interrupt, counted");
    assert_eq!(m.uart, b"OK\nyes", "what the demonstration said and echoed");
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
        let m = lockstep(&p, &format!("random seed {seed}"), Some(seed));
        assert_eq!(m.halted, Some(Halt::Break), "seed {seed} faulted");
    }
}
