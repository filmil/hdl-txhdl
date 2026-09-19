// SPDX-License-Identifier: Apache-2.0
//! The core against the model, every cycle: the model steps when the
//! core retires an instruction, and then the program counter, the
//! thirty-one registers, the control registers and the halt must
//! agree, and the data memory at the end. The demonstration program
//! and a batch of random ones.
use txhdl::comp::{join2, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi_units, AxiHost, AxiPer};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge1};
use txhdl_parts::bus::router::Router3;
use vreteno32::core::{Vreteno, Writeback};
use vreteno32::dmem::Dmem;
use vreteno32::isa::{
    decode, disasm, Kind, CAUSE_MEXT, CAUSE_MSOFT, CAUSE_MTIMER,
};
use vreteno32::model::{Halt, Model};
use vreteno32::program::{demo, random, soft};
use vreteno32::term::Terminal;
use vreteno32::timer::Timer;
use vreteno32::uart::Uart;

/// The link the core sits on, as the demonstration has it.
const IW: usize = 2;
const NIDS: usize = 4;
type Rtr =
    Router3<
    32,
    32,
    4,
    IW,
    0x1000,
    0xf000,
    0x0200_0000,
    0xffff_0000,
    0x3000,
    0xf000,
>;

/// The bridge the serial port sits behind: one AXI-Lite peripheral,
/// at the range the router gives the port.
type Serial = LiteBridge1<32, 32, 4, IW, 0x3000, 0xf000>;

/// Runs `program` on both until the core halts, checking after every
/// cycle; returns the model at the halt.
fn lockstep(program: &[u32], what: &str, seed: Option<u64>) -> Model {
    let mut cpu = Vreteno::with(program);
    let (pc, ir_pc, valid, regs, halted) = (
        cpu.pc,
        cpu.ir_pc,
        cpu.valid,
        cpu.regs.clone(),
        cpu.halted,
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
    let (wb_valid, wb_pc) = (cpu.wb_valid, cpu.wb_pc);
    let csrs = [
        ("mstatus", cpu.mstatus),
        ("mtvec", cpu.mtvec),
        ("mscratch", cpu.mscratch),
        ("mepc", cpu.mepc),
        ("mcause", cpu.mcause),
        ("mie", cpu.mie),
        ("mip", cpu.mip),
        ("mtval", cpu.mtval),
    ];
    let (mip, mie, mstatus) =
        (cpu.mip, cpu.mie, cpu.mstatus);

    let mut timer = Timer::<IW>::default();
    let mut uart = Uart::<4>::default();
    let mtimecmp = timer.mtimecmp;
    let (pending, wb_dev) = (timer.pending, cpu.wb_dev);
    let msip = timer.msip;
    let (uart_sent, uart_last) = (uart.sent, uart.last);
    let (uart_received, uart_dropped) =
        (uart.received, uart.dropped);
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
    let (sirq_out, sirq) = signal::<Bit, DefaultClock>();
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
    // The serial port is an AXI-Lite peripheral, behind a bridge
    // that takes the AXI4 channels the router gives it.
    let sl = axi_lite::<32, 32, 4>();
    let (uaw, uar, uw, ub, ur) = sl.per;
    let (baw, bar, bw, bb, br) = sl.host;
    let mut axi_host = AxiHost::<32, 32, 4, IW, NIDS>::default();
    let mut dper = AxiPer::<32, 32, 4, IW>::default();
    let mut tper = AxiPer::<32, 32, 4, IW>::default();
    let mut ubridge = Serial::default();
    let mut router = Rtr::default();
    let (halt_out, _halt) = signal::<Bit, DefaultClock>();
    let (instr_out, _instr) = signal::<U<32>, DefaultClock>();
    let (ir, in_execute) = (cpu.ir, cpu.valid);
    let stall = cpu.stall.clone();
    let (wb_out, wb) = signal::<Writeback, DefaultClock>();
    // The timer first, since the core reads its line in the same step.
    let (rst_t, rst_u) = (rst.clone(), rst.clone());
    let mut sim = Running::new(join2(
        join2(
            join2(
                timer.run((rst_t, treq, twd), (tans, trb, tirq_out, sirq_out)),
                uart.run((rst_u, rx, uaw, uar, uw), (ub, ur, tx_out, uirq_out)),
            ),
            join2(
                dmem.run((dreq, dwd), (dans, drb)),
                cpu.run(
                    (rst, irq, tirq, sirq, crdata, cdone, grant),
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
                ubridge.run(
                    (ul.per_in.0, ul.per_in.1, ul.per_in.2, bb, br),
                    (baw, bar, bw, ul.per_out.2, ul.per_out.3),
                ),
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
    // The software interrupt's line, the same cycle behind: the
    // controller's `msip` as the core saw it.
    let mut soft = false;
    let mut soft_before = false;
    let mut answer;
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
        let soft_now = msip.get().to_bool();
        let ext = mip.get().bit(11).to_bool() && mie.get().bit(11).to_bool();
        let sft = soft_now && mie.get().bit(3).to_bool();
        let tim = line_now && mie.get().bit(7).to_bool();
        // The order the specification gives, which the core keeps.
        let taken_now = if !mstatus.get().bit(3).to_bool() {
            None
        } else if ext {
            Some(CAUSE_MEXT)
        } else if sft {
            Some(CAUSE_MSOFT)
        } else if tim {
            Some(CAUSE_MTIMER)
        } else {
            None
        };
        sim.cycle();
        term.see(tx.get().to_bool());
        if executing && !stall.get().to_bool() {
            line = line_now;
            soft = soft_now;
            taken = taken_now;
        }
        if wb.get().done.to_bool() {
            model.dev_word = answer;
            model.tirq = line_before;
            model.msip = soft_before;
            model.step(program, taken_before);
            retired += 1;
        }
        taken_before = taken;
        line_before = line;
        soft_before = soft;
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
        // A load that traps writes the CSRs in execute as a system
        // instruction does, so it is skipped for the same reason: the
        // address is the one the core used, since the registers agree.
        let d = decode(executed);
        let bad_access = matches!(
            d.kind,
            Kind::Lb | Kind::Lh | Kind::Lw | Kind::Lbu | Kind::Lhu
        ) && vreteno32::model::misaligned(
            d.kind,
            model.x[d.rs1 as usize].wrapping_add(d.imm as u32),
        );
        let system = executing
            && !stall.get().to_bool()
            && (taken.is_some()
                || bad_access
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
            // The demonstration is the one that types at the port and
            // echoes what it was given.
            if what == "demo" {
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
fn a_program_that_interrupts_itself() {
    // The program raises the software interrupt for itself and its
    // handler clears it, until three have been taken; a return goes
    // back to the store that raised it, so the last round of the loop
    // takes another, and what the program guarantees is three or more.
    // The model takes them where the core does, which is what the
    // lockstep compares cycle by cycle.
    let m = lockstep(&soft(), "soft", None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert!(m.x[8] >= 3, "interrupts taken: {}", m.x[8]);
    assert_eq!(m.mem[0], m.x[8], "and the program wrote what it counted");
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

/// A multiply and a divide whose operand is the word the instruction
/// before them loaded. The load's answer comes back over the bus, so
/// for a while the load sits in writeback with nothing to forward and
/// the register file still holds the old value. The sequencer latches
/// its operands when it starts, so it must not start until the word
/// has landed. It did, and multiplied by the old value: a compiled
/// program found it, when a colour came out with no red in it.
#[test]
fn a_multiply_right_after_a_load() {
    use vreteno32::isa::{addi, div, halt, lui, lw, mul, sw};
    let p = vec![
        lui(6, 1),        // x6 = 0x1000, the data memory
        addi(5, 0, 1234), // the word to load back
        sw(5, 6, 0),
        addi(17, 0, 255),
        addi(7, 0, 10),
        addi(10, 0, 3), // the old value the multiply must not see
        lw(10, 6, 0),
        mul(11, 17, 10),
        addi(12, 0, 3), // and the divide's
        lw(12, 6, 0),
        div(13, 12, 7),
        halt(),
    ];
    let m = lockstep(&p, "a multiply right after a load", Some(1));
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[11], 255 * 1234, "mul of the loaded word");
    assert_eq!(m.x[13], 123, "div of the loaded word");
}

#[test]
fn random_programs() {
    // Instructions by length, and thirty-two bit ones that start in
    // the upper half of a word, read off the programs from the start:
    // the mix the compressed fetch has to get right.
    let (mut short, mut wide, mut straddle) = (0, 0, 0);
    for seed in 0..64 {
        let p = random(seed, 200);
        let mut at = 0;
        while let Some((_, n)) = vreteno32::model::fetch(&p, at) {
            match (n, at % 4) {
                (2, _) => short += 1,
                (_, 2) => straddle += 1,
                _ => wide += 1,
            }
            at += n;
        }
        let m = lockstep(&p, &format!("random seed {seed}"), Some(seed));
        assert_eq!(m.halted, Some(Halt::Break), "seed {seed} faulted");
    }
    let counts = format!("{short} compressed, {wide} whole, {straddle} across");
    assert!(short > 3000, "{counts}");
    assert!(wide > 3000, "{counts}");
    assert!(straddle > 2000, "{counts}");
}

/// The compressed instructions, each at least once, with the lengths
/// mixed so that thirty-two bit instructions start in the upper half of
/// a word: the arithmetic, the stack pointer's short forms, loads and
/// stores, a loop on c.bnez, calls and returns by c.jal, jal, c.jalr and
/// c.jr with their links two or four bytes on, a reserved halfword
/// that traps with itself as the trap value and is stepped over by the
/// handler, and the halt at the end.
#[test]
fn compressed_instructions() {
    use vreteno32::isa::*;
    use vreteno32::program::Asm;
    let mut a = Asm::default();
    let (handler, after, f1, f2, f3, top) = (
        a.label(),
        a.label(),
        a.label(),
        a.label(),
        a.label(),
        a.label(),
    );
    a.wide(lui(2, 1)); // sp = 0x1000, the data memory
    a.abs(handler, |h| addi(31, 0, h as i32));
    a.wide(csrrw(0, CSR_MTVEC, 31));
    a.emit_c(c_li(8, 5)); // x8 = 5
    a.emit_c(c_addi(8, -2)); // x8 = 3
    a.wide(addi(9, 0, 7)); // x9 = 7, starting in an upper half
    a.emit_c(c_nop());
    a.wide(lui(10, 0x12345)); // x10 = 0x12345000
    a.emit_c(c_srli(10, 12)); // x10 = 0x12345
    a.emit_c(c_slli(10, 4)); // x10 = 0x123450
    a.emit_c(c_srai(10, 8)); // x10 = 0x1234
    a.emit_c(c_andi(10, 0x0f)); // x10 = 4
    a.emit_c(c_mv(11, 9)); // x11 = 7
    a.emit_c(c_add(11, 8)); // x11 = 10
    a.emit_c(c_sub(11, 10)); // x11 = 6
    a.emit_c(c_xor(11, 8)); // x11 = 5
    a.emit_c(c_or(11, 10)); // x11 = 5
    a.emit_c(c_and(11, 9)); // x11 = 5
    a.emit_c(c_lui(15, -1)); // x15 = 0xfffff000
    a.emit_c(c_addi16sp(32)); // sp = 0x1020
    a.emit_c(c_addi4spn(12, 8)); // x12 = 0x1028
    a.emit_c(c_addi16sp(-32)); // sp = 0x1000
    a.emit_c(c_swsp(9, 4)); // mem[1] = 7
    a.emit_c(c_lwsp(13, 4)); // x13 = 7
    a.emit_c(c_sw(11, 12, 8)); // mem at 0x1030 = 5
    a.emit_c(c_lw(14, 12, 8)); // x14 = 5
                               // A loop: x8 counts down from 3, x9 counts up.
    a.place(top);
    a.emit_c(c_addi(9, 1));
    a.emit_c(c_addi(8, -1));
    a.to_c(top, |o| c_bnez(8, o)); // x9 = 10 after
    a.to_c(f1, c_jal); // x1 = the next address, f1 adds 100 to x9
    a.to(f2, |o| jal(1, o)); // x1 = four bytes on, f2 adds 1000
    a.wide(auipc(5, 0)); // x5 = this address
    a.emit_c(c_addi(5, 10)); // x5 = f3, ten bytes on
    a.emit_c(c_jalr(5)); // x1 = two bytes on
    a.to_c(after, c_j);
    a.place(f3);
    a.emit_c(c_addi(9, 3)); // x9 += 3
    a.emit_c(c_jr(1));
    a.place(after);
    a.emit_c(0x8002); // c.jr x0: reserved, a trap
    a.emit_c(c_beqz(8, 4)); // taken: over the next halfword
    a.emit_c(c_li(9, 0)); // not reached
    a.emit_c(c_mv(16, 1)); // x16 = the last link
    a.wide(halt());
    a.place(f1);
    a.emit_c(c_addi16sp(16)); // sp moves and comes back
    a.wide(addi(9, 9, 100));
    a.emit_c(c_addi16sp(-16));
    a.emit_c(c_jr(1));
    a.place(f2);
    a.wide(addi(9, 9, 1000));
    a.wide(jalr(0, 1, 0));
    // The handler, at a whole word as mtvec needs: the reserved
    // halfword's cause and value, then on past it, two bytes.
    a.align();
    a.place(handler);
    a.wide(csrrs(20, CSR_MCAUSE, 0));
    a.wide(csrrs(21, CSR_MTVAL, 0));
    a.wide(csrrs(22, CSR_MEPC, 0));
    a.emit_c(c_addi(22, 2));
    a.wide(csrrw(0, CSR_MEPC, 22));
    a.wide(mret());
    let p = a.words();
    let m = lockstep(&p, "compressed instructions", Some(7));
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[8], 0, "the loop's count");
    assert_eq!(m.x[9], 7 + 3 + 100 + 1000 + 3, "the loop and the calls");
    assert_eq!(m.x[10], 4);
    assert_eq!(m.x[11], 5);
    assert_eq!(m.x[12], 0x1028);
    assert_eq!(m.x[13], 7);
    assert_eq!(m.x[14], 5);
    assert_eq!(m.x[15], 0xffff_f000);
    assert_eq!(m.x[2], 0x1000, "sp");
    assert_eq!(m.x[20], CAUSE_ILLEGAL);
    assert_eq!(m.x[21], 0x8002, "the halfword is the trap value");
    assert_eq!(m.x[16], m.x[1]);
    assert_eq!(m.x[1] & 1, 0);
}

/// An unaligned load and an unaligned store trap rather than using the
/// aligned word, which is issue 138. The core and the model are held
/// to the same rule every cycle, and the handler counts the traps and
/// steps past each, so the run ends.
#[test]
fn an_unaligned_access_traps() {
    use vreteno32::isa::*;
    use vreteno32::program::Asm;
    let mut a = Asm::default();
    let handler = a.label();
    a.wide(lui(2, 1)); // x2 = 0x1000, the data memory
    a.abs(handler, |h| addi(31, 0, h as i32));
    a.wide(csrrw(0, CSR_MTVEC, 31));
    a.wide(addi(8, 0, 0)); // x8 counts the traps
    a.wide(addi(3, 0, -2)); // x3 = -2, something to store
                            // A word at an aligned address, which goes through and is read
                            // back, so the run says the ordinary path still works.
    a.wide(sw(3, 2, 0));
    a.wide(lw(4, 2, 0)); // x4 = -2
                         // Then the five that must trap: a word one byte along and two
                         // along, and a half at an odd address, each way.
    a.wide(lw(5, 2, 1));
    a.wide(lw(6, 2, 2));
    a.wide(lh(7, 2, 5));
    a.wide(sh(3, 2, 7));
    a.wide(sw(3, 2, 3));
    // And a byte at an odd address, which never traps.
    a.wide(sb(3, 2, 9));
    a.wide(lb(9, 2, 9)); // x9 = -2
    a.wide(halt());
    // The handler: the cause and the trap value of the last one, the
    // count, and on past the instruction, which is four bytes here.
    a.align();
    a.place(handler);
    a.wide(csrrs(20, CSR_MCAUSE, 0));
    a.wide(csrrs(21, CSR_MTVAL, 0));
    a.wide(addi(8, 8, 1));
    a.wide(csrrs(22, CSR_MEPC, 0));
    a.wide(addi(22, 22, 4));
    a.wide(csrrw(0, CSR_MEPC, 22));
    a.wide(mret());
    let p = a.words();
    let m = lockstep(&p, "an unaligned access", Some(11));
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[4], (-2i32) as u32, "the aligned word went through");
    assert_eq!(m.x[9], (-2i32) as u32, "and a byte at an odd address");
    assert_eq!(m.x[8], 5, "five accesses trapped");
    assert_eq!(m.x[5], 0, "a load that trapped wrote no register");
    assert_eq!(m.x[6], 0);
    assert_eq!(m.x[7], 0);
    assert_eq!(
        m.x[20],
        vreteno32::isa::CAUSE_STORE_MISALIGNED,
        "the last trap was a store's"
    );
    assert_eq!(m.x[21], 0x1003, "and its address is the trap value");
    assert_eq!(
        m.mem[0],
        (-2i32) as u32,
        "the trapping stores wrote nothing"
    );
}
