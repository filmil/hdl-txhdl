// SPDX-License-Identifier: Apache-2.0
//! The core against the model, every cycle: the model steps when the
//! core retires an instruction, and then the program counter, the
//! thirty-one registers, the control registers and the halt must
//! agree, and the data memory at the end. The demonstration program
//! and a batch of random ones.
use std::cell::RefCell;
use txhdl::comp::{join2, signal, DefaultClock, Running, Unit};
use txhdl::map::AddrMap;
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi_units, AxiHost, AxiPer, PerPort};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge, LitePort};
use txhdl_parts::bus::router::Router;
use vreteno32::core::Writeback;
use vreteno32::dmem::Dmem;
use vreteno32::hart::Hart;
use vreteno32::isa::{
    add, addi, beq, csrrs, csrrsi, csrrw, csrrwi, decode, disasm, ebreak, halt,
    jal, jalr, lui, lw, mret, or, sw, Kind, CAUSE_FETCH_ACCESS, CAUSE_MEXT,
    CAUSE_MSOFT, CAUSE_MTIMER, CAUSE_SEXT, CAUSE_SSOFT, CAUSE_STIMER,
    CAUSE_STORE_ACCESS, CSR_DCSR, CSR_MBUSQUIET, CSR_MCAUSE, CSR_MEPC, CSR_MIE,
    CSR_MSTATUS, CSR_MTVAL, CSR_MTVEC, MEXT, MISA, MSOFT, MTIMER, SEXT, SSOFT,
    STIMER,
};
use vreteno32::model::{Halt, Model};
use vreteno32::program::{demo, idle, in_memory, machine_info, random, soft};
use vreteno32::rom::Rom;
use vreteno32::term::Terminal;
use vreteno32::timer::Timer;
use vreteno32::uart::Uart;

/// The link the core sits on, as the demonstration has it.
const IW: usize = 2;
const NIDS: usize = 4;
/// The address map: the data memory at its page, the timer at
/// `0x0200_0000`, the serial port at `0x3000`, and the boot memory at
/// zero, as the board has it, which a fetch under translation reads
/// (issue 1014).
struct RunMap;

impl AddrMap<4> for RunMap {
    const RANGES: [(usize, usize); 4] = [
        (0x1000, 0xf000),
        (0x0200_0000, 0xffff_0000),
        (0x3000, 0xf000),
        (0x0000, 0xffff_f000),
    ];
}

type Rtr = Router<4, RunMap, 32, 32, 4, IW>;

/// The bridge the serial port sits behind: one AXI-Lite peripheral,
/// at the range the router gives the port.
type Serial = LiteBridge<1, SerialMap, 32, 32, 4, IW>;

/// Where the serial port is: a nibble of the address space at 0x3000.
pub struct SerialMap;

impl AddrMap<1> for SerialMap {
    const RANGES: [(usize, usize); 1] = [(0x3000, 0xf000)];
}

/// What a debugger does to the core during a run (issue 154): a halt
/// request at a cycle, held until the core has entered debug mode; a
/// hold of some cycles; a resume, for two cycles, since the core acts
/// on the level once per entry; and that again, once per entry, as
/// many times as `resumes` says. `entries` collects `dcsr` at each
/// entry, so a test can read the causes back.
pub struct DebugPlan {
    pub halt_at: u64,
    pub hold: u64,
    pub resumes: u32,
    pub entries: RefCell<Vec<u32>>,
    /// What the debugger writes through the module while the core is
    /// in debug mode: at the entry numbered, a CSR and its value, as
    /// OpenOCD sets `dcsr.step` or `ebreakm` (issue 972).
    pub writes: Vec<(usize, u32, u32)>,
}

/// Runs `program` on both until the core halts, checking after every
/// cycle; returns the model at the halt.
fn lockstep(
    program: &[u32],
    data: &[u32],
    what: &str,
    seed: Option<u64>,
    dbg: Option<&DebugPlan>,
    reset_at: Option<u64>,
) -> Model {
    lockstep_with(program, data, what, seed, dbg, reset_at, None)
}

/// The same, with the supervisor's external line, `seirq`, high in the
/// cycles `seip` says, as the interrupt controller's second target would
/// hold it (issue 1094).
fn lockstep_with(
    program: &[u32],
    data: &[u32],
    what: &str,
    seed: Option<u64>,
    dbg: Option<&DebugPlan>,
    reset_at: Option<u64>,
    seip: Option<fn(u64) -> bool>,
) -> Model {
    let mut hart = Hart::<2>::with(program);
    let cpu = &hart.core;
    let (pc, ir_pc, valid, regs, halted) =
        (cpu.pc, cpu.ir_pc, cpu.valid, cpu.regs.clone(), cpu.halted);
    let (in_debug, dpc, dcsr) = (cpu.debug, cpu.dpc, cpu.dcsr);
    // The data memory starts with `data` in it, which is how a program
    // that lives above the boot memory gets there.
    let bytes: Vec<u8> = data.iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut dmem = Dmem::<IW>::with(&bytes);
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
        // User and supervisor mode (issue 1012).
        ("medeleg", cpu.medeleg),
        ("mideleg", cpu.mideleg),
        ("mip_sw", cpu.mip_sw),
        ("stvec", cpu.stvec),
        ("sscratch", cpu.sscratch),
        ("sepc", cpu.sepc),
        ("scause", cpu.scause),
        ("stval", cpu.stval),
        ("satp", cpu.satp),
    ];
    let (prv, counteren) = (cpu.prv, cpu.counteren);
    let (mideleg_r, mip_sw_r) = (cpu.mideleg, cpu.mip_sw);
    let (mip, mie, mstatus) = (cpu.mip, cpu.mie, cpu.mstatus);
    // The core's interrupt decision, made a cycle before the take
    // (issue 1331).
    let int_q = cpu.int_q;

    let mut timer = Timer::<IW>::default();
    let mut uart = Uart::<4>::default();
    let mtimecmp = timer.mtimecmp;
    let (pending, wb_dev) = (timer.pending, cpu.wb_dev);
    // The bus's refusals: of the load in writeback, and of a store,
    // which the core raises before the next instruction (issue 417).
    let (wb_err, st_err) = (cpu.wb_err, cpu.st_err);
    let msip = timer.msip;
    let (uart_sent, uart_last) = (uart.sent, uart.last);
    let (uart_received, uart_dropped) = (uart.received, uart.dropped);
    // The architectural program counter, as Vreteno::arch_pc has it:
    // the oldest instruction not yet retired.
    let (dbg_on, dbg_pc) = (cpu.debug, cpu.dpc);
    let arch_pc = move || {
        if dbg_on.get().to_bool() {
            // In debug mode nothing is in flight and the fetch has run
            // ahead: the next instruction is the one at `dpc`.
            dbg_pc.get()
        } else if wb_valid.get().to_bool() {
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
    // Hart 1's lines, which a one-hart design leaves unread (issue 1408).
    let (tirq1_out, _tirq1) = signal::<Bit, DefaultClock>();
    let (sirq1_out, _sirq1) = signal::<Bit, DefaultClock>();
    let (time_out, time) = signal::<U<64>, DefaultClock>();
    // The supervisor's external line, which no controller here drives
    // (issue 1094).
    let (seirq_out, seirq) = signal::<Bit, DefaultClock>();
    // No other host writes the DDR3 here, so nothing is snooped (issue 1275).
    let (_dc_snoop_out, dc_snoop) = signal::<U<9>, DefaultClock>();
    let (tx_out, tx) = signal::<Bit, DefaultClock>();
    let (rx_out, rx) = signal::<Bit, DefaultClock>();
    let (uirq_out, uirq) = signal::<Bit, DefaultClock>();
    // The debugger's requests, driven by the plan, and what the core
    // says about debug mode, read from its register rather than its
    // port, which lags a cycle.
    let (haltreq_out, haltreq) = signal::<Bit, DefaultClock>();
    let (resumereq_out, resumereq) = signal::<Bit, DefaultClock>();
    let (debug_out, _debug) = signal::<Bit, DefaultClock>();
    // The debug module's register access: a plan's writes drive the
    // number, the word and the strobe for a cycle; the answer is unread.
    let (dbg_regno_o, dbg_regno) = signal::<U<16>, DefaultClock>();
    let (dbg_wdata_o, dbg_wdata) = signal::<U<32>, DefaultClock>();
    let (dbg_we_o, dbg_we) = signal::<Bit, DefaultClock>();
    let (dbg_rdata_o, _dbg_rdata) = signal::<U<32>, DefaultClock>();
    // The core's link, and one per peripheral, with the router
    // between the core's tracker and the three peripherals'.
    let cl = axi_units::<32, 32, 4, IW>();
    let dl = axi_units::<32, 32, 4, IW>();
    let tl = axi_units::<32, 32, 4, IW>();
    let ul = axi_units::<32, 32, 4, IW>();
    let rl = axi_units::<32, 32, 4, IW>();
    let rbus = PerPort::from(rl.per_client);
    let mut rom = Rom::<IW>::with(program);
    let mut rper = AxiPer::<32, 32, 4, IW>::default();
    let (issue, wbeat, release, grant, cdone, crdata) = cl.host_client;
    let dbus = PerPort::from(dl.per_client);
    let tbus = PerPort::from(tl.per_client);
    // The serial port is an AXI-Lite peripheral, behind a bridge
    // that takes the AXI4 channels the router gives it.
    let sl = axi_lite::<32, 32, 4>();
    let ubus: LitePort<32, 32, 4> = sl.per.into();
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
                timer.run(
                    tbus,
                    (rst_t, tirq_out, sirq_out, time_out, tirq1_out, sirq1_out),
                ),
                uart.run(ubus, (rst_u, rx, tx_out, uirq_out)),
            ),
            join2(
                dmem.run(dbus, ()),
                hart.run(
                    (
                        rst, irq, tirq, sirq, crdata, cdone, grant, haltreq,
                        resumereq, dbg_regno, dbg_wdata, dbg_we, time, seirq,
                        dc_snoop,
                    ),
                    (
                        halt_out,
                        instr_out,
                        wb_out,
                        issue,
                        wbeat,
                        release,
                        debug_out,
                        dbg_rdata_o,
                    ),
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
                        [
                            dl.host_in.2,
                            tl.host_in.2,
                            ul.host_in.2,
                            rl.host_in.2,
                        ],
                        [
                            dl.host_in.3,
                            tl.host_in.3,
                            ul.host_in.3,
                            rl.host_in.3,
                        ],
                    ),
                    (
                        [
                            dl.host_out.0,
                            tl.host_out.0,
                            ul.host_out.0,
                            rl.host_out.0,
                        ],
                        [
                            dl.host_out.1,
                            tl.host_out.1,
                            ul.host_out.1,
                            rl.host_out.1,
                        ],
                        [
                            dl.host_out.2,
                            tl.host_out.2,
                            ul.host_out.2,
                            rl.host_out.2,
                        ],
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
                join2(
                    ubridge.run(
                        (ul.per_in.0, ul.per_in.1, ul.per_in.2, [bb], [br]),
                        ([baw], [bar], [bw], ul.per_out.2, ul.per_out.3),
                    ),
                    join2(rper.run(rl.per_in, rl.per_out), rom.run(rbus, ())),
                ),
            ),
        ),
    ));
    rst_out.set(Bit::One);
    sim.cycle();
    rst_out.set(Bit::Zero);
    let mut model = Model::default();
    for (i, &w) in data.iter().enumerate() {
        model.mem[i] = w;
    }
    let mut retired = 0;
    // The longest stretch of cycles in which nothing retired, which is
    // what a `wfi` looks like from outside: a core that waits rather
    // than spinning retires nothing at all while it waits.
    let (mut quiet, mut longest_quiet) = (0usize, 0usize);
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
    // The demonstration's line is a device's: raised at cycle 40 and
    // held until the core takes the external interrupt, as the
    // interrupt controller holds its line until it is claimed. Since
    // the pending bit is the line and not a latch (#788), a pulse that
    // drops before it is served is no interrupt at all.
    let mut demo_line = false;
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
    // The supervisor's line as the core read it: the core registers
    // SEIP and reads `mip` in execute, a cycle before the instruction
    // retires, so the line it saw is two cycles behind the model's step,
    // where the model reads its own `mip` (issue 1295).
    let (mut s_before, mut s_before2) = (false, false);
    // The pending and enabled set as the core chose from it a cycle
    // ago: it acts on that decision now, unless something since has
    // changed what may be taken, which its `int_q` says (issue 1331).
    let mut set_before = 0u32;
    let mut answer;
    // The terminal on the port's lines: it answers the demonstration's
    // line with three bytes, which the program echoes; a random program
    // gets nothing typed. The port's interrupt joins the core's line,
    // as it does on the board.
    let mut term = Terminal::new(if seed.is_none() { b"yes" } else { b"" });
    // The debugger's state, from the plan: cycles held in debug mode
    // so far, resumes left, and the cycles left of the resume pulse.
    let (mut held, mut resumes_left, mut resume_pulse) =
        (0u64, dbg.map_or(0, |p| p.resumes), 0u8);
    for cycle in 0..32768 {
        let at = model.pc;
        let pulse = match seed {
            None => {
                if cycle == 40 {
                    demo_line = true;
                }
                demo_line
            }
            Some(_) => {
                noise ^= noise << 13;
                noise ^= noise >> 7;
                noise ^= noise << 17;
                noise & 15 == 0
            }
        };
        let raised = pulse || uirq.get().to_bool();
        irq_out.set(raised);
        let s_raised = seip.is_some_and(|f| f(cycle as u64));
        seirq_out.set(Bit::from_bool(s_raised));
        rx_out.set(term.level());
        // The debugger's lines for this cycle: the halt request from
        // its cycle until the core is in debug mode, and a resume for
        // two cycles once the hold has passed, since the core acts on
        // the level once per entry and the line must drop between.
        let debugging = in_debug.get().to_bool();
        let (mut hreq, mut rreq) = (false, false);
        let mut dwrite: Option<(u32, u32)> = None;
        if let Some(plan) = dbg {
            if debugging {
                held += 1;
                // The entry's write, on its first cycle in debug mode.
                if held == 1 {
                    let entry = plan.entries.borrow().len().saturating_sub(1);
                    dwrite = plan
                        .writes
                        .iter()
                        .find(|(e, _, _)| *e == entry)
                        .map(|&(_, c, v)| (c, v));
                }
                if held > plan.hold && resumes_left > 0 && resume_pulse == 0 {
                    resume_pulse = 2;
                    resumes_left -= 1;
                }
            } else {
                held = 0;
            }
            hreq = !debugging
                && cycle as u64 >= plan.halt_at
                && plan.entries.borrow().is_empty();
            if resume_pulse > 0 {
                rreq = true;
                resume_pulse -= 1;
            }
        }
        haltreq_out.set(hreq);
        resumereq_out.set(rreq);
        let (wc, wv) = dwrite.unwrap_or((0, 0));
        dbg_regno_o.set(U::from(wc as u16));
        dbg_wdata_o.set(U::from(wv));
        dbg_we_o.set(Bit::from_bool(dwrite.is_some()));
        // The reset line, high for the one cycle the plan names: the
        // core and the devices see it in this cycle, and the model is
        // reset after it, once whatever retired in it has been stepped.
        let resetting = reset_at == Some(cycle as u64);
        if resetting {
            rst_out.set(Bit::One);
        }
        // The word about to execute this cycle, or zero on a bubble;
        // the illegal word is zero too, so the flag is kept apart.
        let executing = in_execute.get().to_bool();
        let executed = if executing { ir.get().raw() as u32 } else { 0 };
        // The register holds the load's answer until the cycle in
        // which it retires, which is the next instruction's execute
        // cycle, so it is read before that cycle.
        answer = wb_dev.get().raw() as u32;
        let refused = wb_err.get().to_bool();
        let line_now = pending.get().to_bool();
        let soft_now = msip.get().to_bool();
        // The interrupt the core takes, by its rule (issue 1012): the
        // pending and enabled ones machine mode keeps, below machine
        // mode or with MIE, before those it delegated, below
        // supervisor mode or in it with SIE; within each, the
        // external, the software and the timer's, the machine's
        // first. A refused store is taken first, and whether or not
        // interrupts are enabled: it is a trap, not an interrupt.
        let pend = (mip.get().raw() as u32
            | if line_now { MTIMER } else { 0 }
            | if soft_now { MSOFT } else { 0 }
            | mip_sw_r.get().raw() as u32)
            & mie.get().raw() as u32;
        let (p, st) = (prv.get().raw() as u32, mstatus.get().raw() as u32);
        let dl = mideleg_r.get().raw() as u32;
        let m_set = if p != 3 || st & 8 != 0 { pend & !dl } else { 0 };
        let s_on = p == 0 || (p == 1 && st & 2 != 0);
        let s_set = if s_on { pend & dl } else { 0 };
        let set = if m_set != 0 { m_set } else { s_set };
        let decided = int_q.get().to_bool();
        assert!(
            !decided || set_before != 0,
            "the core decided on an interrupt with none pending and enabled"
        );
        let taken_now = if st_err.get().to_bool() {
            Some(CAUSE_STORE_ACCESS)
        } else if !decided {
            None
        } else {
            [
                (MEXT, CAUSE_MEXT),
                (MSOFT, CAUSE_MSOFT),
                (MTIMER, CAUSE_MTIMER),
                (SEXT, CAUSE_SEXT),
                (SSOFT, CAUSE_SSOFT),
                (STIMER, CAUSE_STIMER),
            ]
            .into_iter()
            .find(|&(bit, _)| set_before & bit != 0)
            .map(|(_, cause)| cause)
        };
        sim.cycle();
        term.see(tx.get().to_bool());
        if let Some((c, v)) = dwrite {
            model.debug_write(c, v);
        }
        if executing && !stall.get().to_bool() {
            line = line_now;
            soft = soft_now;
            taken = taken_now;
            // Served: the device lets its line go.
            if taken_now == Some(CAUSE_MEXT) {
                demo_line = false;
            }
        }
        if wb.get().done.to_bool() {
            model.dev_word = answer;
            model.dev_err = refused;
            model.tirq = line_before;
            model.msip = soft_before;
            model.sline(s_before2);
            // An interrupt the core takes must be one the architecture
            // allows at this boundary, as the model's state stands after
            // everything before it: the core decides a cycle early, and
            // drops the decision after anything that changes the enables
            // (issue 1331); this is what says it dropped it when it had to.
            if let Some(cause) = taken_before.filter(|c| c & 0x8000_0000 != 0) {
                let bit = 1u32 << (cause & 31);
                let (p, st) = (model.prv, model.csr.mstatus);
                let allowed = model.csr.mie & bit != 0
                    && if model.csr.mideleg & bit != 0 {
                        p == 0 || (p == 1 && st & 2 != 0)
                    } else {
                        p != 3 || st & 8 != 0
                    };
                assert!(
                    allowed,
                    "{what}, cycle {cycle}: interrupt {cause:#x} taken with \
                     prv {p}, mstatus {st:#x}, mie {:#x}, mideleg {:#x}",
                    model.csr.mie, model.csr.mideleg
                );
            }
            model.step(program, taken_before);
            retired += 1;
            quiet = 0;
        } else {
            quiet += 1;
            longest_quiet = longest_quiet.max(quiet);
        }
        taken_before = taken;
        set_before = set;
        line_before = line;
        soft_before = soft;
        s_before2 = s_before;
        s_before = s_raised;
        // Debug mode is entered before the instruction in execute,
        // after the one behind it retired, and left to `dpc`; the
        // model follows the core's register at each edge, and the
        // plan keeps `dcsr` as it was written at the entry.
        let debugging_now = in_debug.get().to_bool();
        if debugging_now && !debugging {
            model.enter_debug(program);
            if let Some(plan) = dbg {
                plan.entries.borrow_mut().push(dcsr.get().raw() as u32);
            }
        } else if debugging && !debugging_now {
            model.resume();
        }
        if resetting {
            rst_out.set(Bit::Zero);
            model.reset();
        }
        // The pending bit is the line, taken at this edge in both.
        model.line(raised);
        model.sline(s_raised);
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
        // Under translation any load may trap, on its page (issue
        // 1014), and a load is translated in machine mode too while
        // `MPRV` names another mode (issue 1105).
        let d = decode(executed);
        let st = model.csr.mstatus;
        let dprv = if model.prv == 3 && st >> 17 & 1 == 1 {
            st >> 11 & 3
        } else {
            model.prv
        };
        let translating = model.csr.satp >> 31 == 1 && dprv != 3;
        let bad_access = matches!(
            d.kind,
            Kind::Lb | Kind::Lh | Kind::Lw | Kind::Lbu | Kind::Lhu
        ) && (translating
            || vreteno32::model::misaligned(
                d.kind,
                model.x[d.rs1 as usize].wrapping_add(d.imm as u32),
            ));
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
                        | Kind::Sret
                        | Kind::Wfi
                        // An illegal one under `TVM` traps in execute,
                        // before the model steps it (issue 1347).
                        | Kind::SfenceVma
                        | Kind::Illegal
                        | Kind::Sb
                        | Kind::Sh
                        | Kind::Sw
                        | Kind::LrW
                        | Kind::ScW
                        | Kind::AmoswapW
                        | Kind::AmoaddW
                        | Kind::AmoxorW
                        | Kind::AmoandW
                        | Kind::AmoorW
                        | Kind::AmominW
                        | Kind::AmomaxW
                        | Kind::AmominuW
                        | Kind::AmomaxuW
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
            model.csr.medeleg,
            model.csr.mideleg,
            model.csr.mip_sw,
            model.csr.stvec,
            model.csr.sscratch,
            model.csr.sepc,
            model.csr.scause,
            model.csr.stval,
            model.csr.satp,
        ];
        for ((name, r), w) in csrs.iter().zip(want) {
            if !system {
                assert_eq!(r.get().raw() as u32, w, "{name} after {here}");
            }
        }
        if !system {
            assert_eq!(prv.get().raw() as u32, model.prv, "prv after {here}");
            assert_eq!(
                counteren.get().raw() as u32,
                model.csr.mcounteren | model.csr.scounteren << 3,
                "counteren after {here}"
            );
            assert_eq!(dpc.get().raw() as u32, model.dpc, "dpc after {here}");
            assert_eq!(
                dcsr.get().raw() as u32,
                model.dcsr,
                "dcsr after {here}"
            );
        }
        assert_eq!(debugging_now, model.debug, "debug mode after {here}");
        // A plan with no resume left ends the run in debug mode, held
        // there: the program does not halt, and the memory and the
        // devices are not checked.
        if let Some(plan) = dbg {
            if debugging_now && resumes_left == 0 && held > plan.hold {
                return model;
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
            // The idle program stops at a `wfi` until the timer wakes
            // it. A core that spun instead would retire something in
            // nearly every cycle, so the long silence is the evidence
            // that the wait is a wait.
            if what == "idle" {
                assert!(
                    longest_quiet > 100,
                    "idle: the longest silence was {longest_quiet} cycles, \
                     which is a core that spun rather than waited"
                );
            }
            return model;
        }
    }
    panic!(
        "{what}: no halt in 32768 cycles; retired {retired}, model pc \
         {:#x}, causes {:?}, mcause {} mepc {:#x} mtval {:#x} scause {} \
         sepc {:#x} stval {:#x} prv {}",
        model.pc,
        model.causes,
        model.csr.mcause,
        model.csr.mepc,
        model.csr.mtval,
        model.csr.scause,
        model.csr.sepc,
        model.csr.stval,
        model.prv
    );
}

#[test]
fn a_program_that_interrupts_itself() {
    // The program raises the software interrupt for itself and its
    // handler clears it, until three have been taken; a return goes
    // back to the store that raised it, so the last round of the loop
    // takes another, and what the program guarantees is three or more.
    // The model takes them where the core does, which is what the
    // lockstep compares cycle by cycle.
    let m = lockstep(&soft(), &[], "soft", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert!(m.x[8] >= 3, "interrupts taken: {}", m.x[8]);
    assert_eq!(m.mem[0], m.x[8], "and the program wrote what it counted");
}

#[test]
fn a_program_reads_what_the_machine_says_it_is() {
    // `mhartid` is the first thing a stock kernel reads, and it used to
    // trap. The program reads it and `misa`, then writes to a read-only
    // register, which is an illegal instruction its handler counts.
    // The model answers all of it the same way, which the lockstep
    // compares every cycle rather than only at the end.
    let m = lockstep(&machine_info(), &[], "machine info", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.mem[0], 0, "mhartid, this machine's one hart");
    assert_eq!(m.mem[1], MISA, "misa: RV32IMAC");
    assert_eq!(
        m.mem[2], 1,
        "one illegal instruction, the write to a read-only register"
    );
    // The letters, spelled out, so the number above is not the only
    // thing that says what the core is.
    assert_eq!(m.mem[1] >> 30, 1, "MXL: a 32-bit machine");
    for (bit, letter) in [(8, 'I'), (12, 'M'), (2, 'C'), (0, 'A')] {
        assert!(m.mem[1] & (1 << bit) != 0, "misa should have {letter}");
    }
    for (bit, letter) in [(5, 'F'), (3, 'D')] {
        assert!(m.mem[1] & (1 << bit) == 0, "misa should not have {letter}");
    }
}

#[test]
fn a_program_that_waits_for_an_interrupt_is_woken_by_one() {
    // The program arms the timer and stops at a `wfi` until the line
    // comes up, twice. The second wait runs with interrupts disabled
    // globally, since the handler returns with `mstatus.MIE` as `mret`
    // leaves it, and the core wakes from it anyway: a wait ends when
    // an interrupt is pending and enabled, whether or not it may be
    // taken, which is what lets a kernel idle inside its own lock.
    let m = lockstep(&idle(), &[], "idle", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert!(m.x[8] >= 2, "interrupts that woke it: {}", m.x[8]);
    assert_eq!(m.mem[0], m.x[8], "and the program wrote what it counted");
}

#[test]
fn a_program_above_the_boot_memory_is_fetched_from_the_bus() {
    // The boot memory holds a jump into the data memory, and the
    // program itself is in the data memory, so every instruction after
    // the jump comes back over the bus. It adds the first ten numbers
    // and writes the sum where the test can read it.
    let (boot, prog) = in_memory();
    let m = lockstep(&boot, &prog, "in memory", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[10], 55, "the sum the program computed");
    assert_eq!(m.mem[16], 55, "and wrote at offset 64");
}

#[test]
fn demo_program() {
    let m = lockstep(&demo(), &[], "demo", None, None, None);
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
    let m = lockstep(
        &p,
        &[],
        "a multiply right after a load",
        Some(1),
        None,
        None,
    );
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[11], 255 * 1234, "mul of the loaded word");
    assert_eq!(m.x[13], 123, "div of the loaded word");
}

/// The data RAM on the core's own port (issue 1275), at 0x1_0000, in
/// lockstep with the model, which keeps the window as memory of its own.
/// A store and a load of the same word at once; a load whose address is
/// the word a load before it gave, which the two steps of the
/// simulation must agree on; bytes and halves under the strobes; the
/// window's compare on both its arms, an offset below 0x2_0000 and one
/// that carries in from below 0x1_0000; and lr/sc and an AMO there.
#[test]
fn the_data_ram_on_the_cores_own_port() {
    use vreteno32::isa::{
        addi, amoadd_w, halt, lb, lhu, lr_w, lui, lw, sb, sc_w, sh, sw,
    };
    let p = vec![
        lui(6, 0x10), // x6 = 0x1_0000, the window
        addi(5, 0, 0x123),
        sw(5, 6, 0),
        lw(10, 6, 0), // the word stored the cycle before
        sw(6, 6, 4),  // a pointer to 0x1_0000, at 0x1_0004
        lw(12, 6, 4),
        lw(13, 12, 0), // through the pointer just loaded
        addi(7, 0, -1),
        sw(0, 6, 8),
        sb(7, 6, 8),
        sh(7, 6, 10),
        lw(14, 6, 8),   // 0xffff_00ff
        lb(15, 6, 8),   // -1
        lhu(16, 6, 10), // 0xffff
        lui(8, 0x20),   // x8 = 0x2_0000, past the window
        addi(5, 0, 77),
        sw(5, 8, -4), // 0x1_fffc: a negative offset into it
        lw(17, 8, -4),
        lui(9, 0x10),
        addi(9, 9, -4), // x9 = 0xfffc, below it
        lw(18, 9, 8),   // 0x1_0004: a carry into it, the pointer
        lr_w(19, 6),    // 0x123
        sc_w(20, 6, 5), // 0, stored
        lw(21, 6, 0),   // 77
        addi(5, 0, 5),
        amoadd_w(22, 6, 5), // 77, and 82 stored
        lw(23, 6, 0),       // 82
        halt(),
    ];
    let m = lockstep(&p, &[], "the data RAM", Some(1), None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[10], 0x123, "the word stored the cycle before");
    assert_eq!(m.x[13], 0x123, "through the pointer just loaded");
    assert_eq!(m.x[14], 0xffff_00ff, "a byte and a half under strobes");
    assert_eq!(m.x[15], u32::MAX, "a byte, signed");
    assert_eq!(m.x[16], 0xffff, "a half, unsigned");
    assert_eq!(m.x[17], 77, "a negative offset into the window");
    assert_eq!(m.x[18], 0x1_0000, "a carry into the window");
    assert_eq!(m.x[19], 0x123, "lr");
    assert_eq!(m.x[20], 0, "sc stored");
    assert_eq!(m.x[21], 77, "what sc stored");
    assert_eq!(m.x[22], 77, "the AMO's old word");
    assert_eq!(m.x[23], 82, "the AMO's new word");
}

#[test]
fn random_programs() {
    // Instructions by length, and thirty-two bit ones that start in
    // the upper half of a word, read off the programs from the start:
    // the mix the compressed fetch has to get right.
    let (mut short, mut wide, mut straddle) = (0, 0, 0);
    // Runs that took a trap to the supervisor, and of those whose last
    // was a delegated interrupt (issue 1012).
    let (mut s_traps, mut s_ints) = (0, 0);
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
        let m = lockstep(
            &p,
            &[],
            &format!("random seed {seed}"),
            Some(seed),
            None,
            None,
        );
        assert_eq!(m.halted, Some(Halt::Break), "seed {seed} faulted");
        if m.csr.scause != 0 {
            s_traps += 1;
            if m.csr.scause >> 31 == 1 {
                s_ints += 1;
            }
        }
    }
    assert!(s_traps >= 8, "delegated traps in {s_traps} runs");
    assert!(s_ints >= 2, "delegated interrupts last in {s_ints} runs");
    let counts = format!("{short} compressed, {wide} whole, {straddle} across");
    // The atomics, which have no compressed spelling, took some of the
    // slots in issue 1010: 2876 compressed then, against 3000 before.
    assert!(short > 2500, "{counts}");
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
    let m = lockstep(&p, &[], "compressed instructions", Some(7), None, None);
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
    let m = lockstep(&p, &[], "an unaligned access", Some(11), None, None);
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

/// The cause field of a `dcsr` value, bits 8 to 6.
fn cause(dcsr: u32) -> u32 {
    (dcsr >> 6) & 7
}

/// A halt request in the middle of the demonstration stops the core
/// before an instruction, holds it, and the resume runs the rest:
/// the program ends as it does without the debugger, and the one entry
/// says a halt request was the cause.
#[test]
fn debug_halt_and_resume() {
    let plan = DebugPlan {
        halt_at: 200,
        hold: 20,
        resumes: 1,
        entries: RefCell::new(Vec::new()),
        writes: Vec::new(),
    };
    let m = lockstep(&demo(), &[], "demo, halted", None, Some(&plan), None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[10], 110);
    assert_eq!(m.uart, b"OK\nyes", "what the demonstration said and echoed");
    let entries = plan.entries.borrow();
    assert_eq!(entries.len(), 1, "one entry, on the request");
    assert_eq!(cause(entries[0]), 3, "the cause is the halt request");
    assert!(!m.debug, "the core ran on after the resume");
}

/// With `dcsr.step` set, every resume runs one instruction and the
/// core is back in debug mode with the step as the cause, until the
/// debugger clears the bit, after which a resume runs it to the end.
/// The debugger sets and clears it through the debug module, since
/// the program may not name `dcsr` (issue 972).
#[test]
fn debug_single_steps() {
    let mut p: Vec<u32> = (0..40).map(|_| addi(1, 1, 1)).collect();
    p.extend((0..4).map(|_| addi(2, 2, 1)));
    p.push(halt());
    // Set at the request's entry, cleared at the tenth step's.
    let plan = DebugPlan {
        halt_at: 20,
        hold: 3,
        resumes: 60,
        entries: RefCell::new(Vec::new()),
        writes: vec![(0, CSR_DCSR, 4), (10, CSR_DCSR, 0)],
    };
    let m = lockstep(&p, &[], "single steps", None, Some(&plan), None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[1], 40, "every step ran exactly one instruction");
    assert_eq!(m.x[2], 4);
    let entries = plan.entries.borrow();
    assert_eq!(entries.len(), 11, "entries: {entries:x?}");
    assert_eq!(cause(entries[0]), 3, "the first entry is the request");
    assert_eq!(entries[0] & 4, 0, "the bit is set after that entry");
    for (i, &e) in entries.iter().enumerate().skip(1) {
        assert_eq!(cause(e), 4, "entry {i} is a step: {e:#x}");
        assert_eq!(e & 4, 4, "step bit, entry {i}");
    }
    assert_eq!(m.dcsr & 4, 0, "the debugger cleared the step bit");
}

/// With `dcsr.ebreakm` set, `ebreak` enters debug mode instead of
/// trapping: `dpc` is its address, the cause is the breakpoint, and
/// the instruction before it ran. The debugger halts the core at the
/// start to set the bit through the debug module and resumes it once,
/// so the run ends held in debug mode at the breakpoint.
#[test]
fn debug_ebreak() {
    let p = [addi(1, 0, 7), ebreak(), addi(1, 0, 9), halt()];
    let plan = DebugPlan {
        halt_at: 0,
        hold: 10,
        resumes: 1,
        entries: RefCell::new(Vec::new()),
        writes: vec![(0, CSR_DCSR, 0x8000)],
    };
    let m = lockstep(&p, &[], "ebreak", None, Some(&plan), None);
    assert!(m.debug, "held in debug mode");
    assert_eq!(m.halted, None, "no trap and no halt");
    assert_eq!(m.x[1], 7, "the instruction before the breakpoint ran");
    assert_eq!(m.dpc, 4, "dpc is the breakpoint's address");
    let entries = plan.entries.borrow();
    assert_eq!(entries.len(), 2, "entries: {entries:x?}");
    assert_eq!(cause(entries[0]), 3, "the first is the request");
    assert_eq!(cause(entries[1]), 1, "the second is the breakpoint");
    assert_eq!(entries[1] & 0x8000, 0x8000, "ebreakm stays set");
}

/// Outside debug mode `dcsr` and `dpc` are not there: reading either,
/// and writing either, is an illegal instruction in the core and the
/// model alike, and the write changes nothing (issue 972).
#[test]
fn debug_csrs_trap_outside_debug_mode() {
    use vreteno32::isa::*;
    use vreteno32::program::Asm;
    let mut a = Asm::default();
    let handler = a.label();
    a.abs(handler, |h| addi(31, 0, h as i32));
    a.wide(csrrw(0, CSR_MTVEC, 31));
    a.wide(addi(8, 0, 0)); // x8 counts the traps
    a.wide(addi(3, 0, -1));
    a.wide(csrrs(5, CSR_DCSR, 0));
    a.wide(csrrs(6, CSR_DPC, 0));
    a.wide(csrrw(0, CSR_DCSR, 3));
    a.wide(csrrw(0, CSR_DPC, 3));
    a.wide(csrrsi(0, CSR_DCSR, 4));
    a.wide(halt());
    // The handler: the cause, the count, and on past the instruction.
    a.align();
    a.place(handler);
    a.wide(csrrs(20, CSR_MCAUSE, 0));
    a.wide(addi(8, 8, 1));
    a.wide(csrrs(22, CSR_MEPC, 0));
    a.wide(addi(22, 22, 4));
    a.wide(csrrw(0, CSR_MEPC, 22));
    a.wide(mret());
    let p = a.words();
    let m = lockstep(&p, &[], "dcsr from machine mode", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[8], 5, "every access trapped");
    assert_eq!(m.x[20], CAUSE_ILLEGAL, "as an illegal instruction");
    assert_eq!(m.x[5], 0, "a read that trapped wrote no register");
    assert_eq!(m.x[6], 0);
    assert_eq!(m.dcsr & 0x8004, 0, "the writes changed nothing");
    assert_eq!(m.dpc, 0);
}

/// The reset line in the middle of a program that has set its trap
/// vector and enabled an interrupt: the core starts again at zero with
/// the CSRs as configuration left them, agreeing with the model in
/// every cycle, and the register file keeps what it held, which is how
/// the program knows it is its second start and halts (issue 419).
#[test]
fn a_reset_puts_the_csrs_back_and_keeps_the_registers() {
    let p = [
        addi(2, 2, 1), // starts, in a register the reset leaves alone
        addi(3, 0, 2),
        beq(2, 3, 8 * 4), // the second start halts
        lui(4, 0x40000),
        csrrw(0, CSR_MTVEC, 4),
        addi(5, 0, 0x80),
        csrrs(0, CSR_MIE, 5),
        csrrsi(0, CSR_MSTATUS, 8),
        addi(1, 1, 1),
        jal(0, -4),
        halt(),
    ];
    let m = lockstep(&p, &[], "a reset mid-program", None, None, Some(30));
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[2], 2, "the program started twice");
    assert!(m.x[1] > 0, "the loop ran before the reset");
    assert_eq!(m.csr.mtvec, 0, "the trap vector is back to zero");
    assert_eq!(m.csr.mie, 0, "the enables are clear");
    assert_eq!(m.csr.mstatus, 0, "interrupts are disabled");
    assert_eq!(m.mtimecmp, u64::MAX, "the timer's compare is all ones");
}

/// A load from an address nothing decodes is refused by the router,
/// and the core traps on it as it retires: a load access fault with
/// the address in `mtval`, the register unwritten, and the instruction
/// after it run once the handler returns. A refused store is a store
/// access fault taken before the next instruction to run, once the
/// answer is back, without an address. With `mbusquiet` set neither
/// traps: the load reads the zero the bus answered and the store is
/// dropped (issue 417).
#[test]
fn a_refused_load_or_store_traps_unless_told_to_be_quiet() {
    let handler = 15 * 4;
    let p = [
        addi(6, 0, handler),
        csrrw(0, CSR_MTVEC, 6),
        lui(4, 0x3000), // 0x0300_0000: nobody's
        lw(5, 4, 0),    // refused: a load access fault, then on
        addi(7, 0, 1),
        sw(0, 4, 0),   // refused, later: a store access fault
        lui(2, 0x1),   // the data memory
        lw(11, 2, 0),  // waits, and the store's answer comes back first
        addi(8, 0, 1), // the store's fault is taken before this, or earlier
        csrrwi(0, CSR_MBUSQUIET, 1),
        lw(12, 4, 0), // quiet: reads the zero the bus answered
        sw(0, 4, 0),  // quiet: dropped
        lw(11, 2, 0), // waits for that answer too
        addi(9, 0, 1),
        halt(),
        // The handler, at 60: counts, sums the causes, keeps the trap
        // values, and steps past a load; a store's fault returns to
        // the instruction it was taken before.
        csrrs(23, CSR_MCAUSE, 0),
        csrrs(24, CSR_MTVAL, 0),
        addi(25, 25, 1),
        add(28, 28, 23),
        or(29, 29, 24),
        addi(26, 0, 7),
        beq(23, 26, 4 * 4),
        csrrs(27, CSR_MEPC, 0),
        addi(27, 27, 4),
        csrrw(0, CSR_MEPC, 27),
        mret(),
    ];
    let m = lockstep(&p, &[], "refused accesses", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[25], 2, "two faults");
    assert_eq!(
        m.x[28],
        5 + 7,
        "a load access fault and a store access fault"
    );
    assert_eq!(
        m.x[29], 0x0300_0000,
        "the load's address; the store has none"
    );
    assert_eq!(m.x[5], 0, "the refused load wrote nothing");
    assert_eq!(m.x[7], 1, "the instruction after the load ran, once");
    assert_eq!(m.x[8], 1, "and the one the store's fault was taken before");
    assert_eq!(m.x[12], 0, "quiet: the zero the bus answered");
    assert_eq!(m.x[9], 1, "quiet: nothing trapped");
    assert!(m.csr.busquiet, "the bit stays set");
}

/// A jump into an address nothing decodes: the fetch is refused, the
/// word comes back zero, and the core raises the instruction access
/// fault with the address in `mtval` rather than an illegal
/// instruction; the handler returns to the link (issue 423).
#[test]
fn a_fetch_from_nowhere_is_an_instruction_access_fault() {
    let handler = 6 * 4;
    let p = [
        addi(6, 0, handler),
        csrrw(0, CSR_MTVEC, 6),
        lui(4, 0x3000), // 0x0300_0000: nobody's
        jalr(1, 4, 0),  // into it, with the link in x1
        addi(7, 0, 1),  // run once the handler returns to the link
        halt(),
        csrrs(23, CSR_MCAUSE, 0),
        csrrs(24, CSR_MTVAL, 0),
        csrrw(0, CSR_MEPC, 1),
        mret(),
    ];
    let m = lockstep(&p, &[], "a fetch from nowhere", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[23], CAUSE_FETCH_ACCESS, "the cause names the fetch");
    assert_eq!(m.x[24], 0x0300_0000, "the address that was fetched");
    assert_eq!(m.x[7], 1, "and the program went on");
}

/// The A extension's word forms against the data memory (issue 1010):
/// each AMO returns the old word and leaves its result, `rd` of zero
/// writes nothing and still stores, an AMO whose operand is its own
/// address register still works, `lr.w` and `sc.w` on one word store
/// and write 0, and an `sc.w` with the reservation used up stores
/// nothing and writes 1. The core and the model agree every cycle.
#[test]
fn atomics_read_modify_and_write_one_word() {
    use vreteno32::isa::*;
    let mut p = vec![lui(2, 0x1)]; // x2 = 0x1000, the data memory
                                   // The word at 0x1000 + 4k holds 10 + k; x5 = -3, x6 = 7.
    for k in 0..12 {
        p.push(addi(7, 0, 10 + k));
        p.push(sw(7, 2, 4 * k));
    }
    p.push(addi(5, 0, -3));
    p.push(addi(6, 0, 7));
    let ops: [fn(u32, u32, u32) -> u32; 9] = [
        amoswap_w, amoadd_w, amoxor_w, amoand_w, amoor_w, amomin_w, amomax_w,
        amominu_w, amomaxu_w,
    ];
    for (k, op) in ops.iter().enumerate() {
        let r = 10 + k as u32;
        p.push(addi(29, 2, 4 * k as i32));
        p.push(op(r, 29, if k % 2 == 0 { 5 } else { 6 }));
    }
    // rd zero, and an operand that is the address register itself.
    p.push(addi(29, 2, 36));
    p.push(amoadd_w(0, 29, 6));
    p.push(addi(28, 2, 40));
    p.push(amoswap_w(19, 28, 28));
    // A reservation used once, then gone.
    p.push(addi(29, 2, 44));
    p.push(lr_w(20, 29));
    p.push(sc_w(21, 29, 6));
    p.push(sc_w(22, 29, 5));
    p.push(halt());
    let m = lockstep(&p, &[], "atomics", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    let old: Vec<u32> = (10..19).map(|r| m.x[r]).collect();
    assert_eq!(old, (10..19).collect::<Vec<u32>>(), "each AMO's old word");
    let m5 = (-3i32) as u32;
    let want = [
        m5,      // swap -3
        11 + 7,  // add 7
        12 ^ m5, // xor -3
        13 & 7,  // and 7
        14 | m5, // or -3
        7,       // min(15, 7)
        16,      // max(16, -3)
        7,       // minu(17, 7)
        m5,      // maxu(18, -3)
    ];
    assert_eq!(&m.mem[0..9], &want, "each AMO's result");
    assert_eq!(m.mem[9], 19 + 7, "rd zero still stores");
    assert_eq!(m.x[19], 20, "the old word, through its own address");
    assert_eq!(m.mem[10], 0x1000 + 40, "the address register stored");
    assert_eq!(m.x[20], 21, "lr.w reads the word");
    assert_eq!(m.x[21], 0, "sc.w with the reservation stores");
    assert_eq!(m.x[22], 1, "sc.w without it does not");
    assert_eq!(m.mem[11], 7, "only the first sc.w stored");
}

/// A misaligned A instruction traps (issue 1010): `lr.w` as a load,
/// cause 4, and `sc.w` and an AMO as a store, cause 6, each with the
/// address as the trap value; none of them writes its register or the
/// memory, and `sc.w`'s reservation is used up all the same.
#[test]
fn a_misaligned_atomic_traps() {
    use vreteno32::isa::*;
    use vreteno32::program::Asm;
    let mut a = Asm::default();
    let handler = a.label();
    a.wide(lui(2, 1)); // x2 = 0x1000, the data memory
    a.abs(handler, |h| addi(31, 0, h as i32));
    a.wide(csrrw(0, CSR_MTVEC, 31));
    a.wide(addi(8, 0, 0)); // x8 counts the traps
    a.wide(addi(9, 0, 0)); // x9 sums the causes
    a.wide(addi(6, 0, 7));
    a.wide(sw(6, 2, 0)); // the word at 0x1000 is 7
    a.wide(addi(28, 2, 2));
    a.wide(lr_w(10, 28)); // cause 4
    a.wide(addi(28, 2, 1));
    a.wide(sc_w(11, 28, 6)); // cause 6
    a.wide(addi(28, 2, 3));
    a.wide(amoadd_w(12, 28, 6)); // cause 6
    a.wide(lw(13, 2, 0)); // still 7
    a.wide(halt());
    a.align();
    a.place(handler);
    a.wide(csrrs(20, CSR_MCAUSE, 0));
    a.wide(csrrs(21, CSR_MTVAL, 0));
    a.wide(add(9, 9, 20));
    a.wide(addi(8, 8, 1));
    a.wide(csrrs(22, CSR_MEPC, 0));
    a.wide(addi(22, 22, 4));
    a.wide(csrrw(0, CSR_MEPC, 22));
    a.wide(mret());
    let m = lockstep(&a.words(), &[], "misaligned atomics", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[8], 3, "three traps");
    assert_eq!(m.x[9], 4 + 6 + 6, "a load's cause, then a store's twice");
    assert_eq!(m.x[21], 0x1003, "the last trap value is the address");
    assert_eq!([m.x[10], m.x[11], m.x[12]], [0, 0, 0], "nothing written");
    assert_eq!(m.x[13], 7, "and the word unchanged");
}

/// User and supervisor mode (issue 1012). Machine mode delegates an
/// environment call from user mode and an illegal instruction, enables
/// `instret` below it, and returns into supervisor mode, which reads
/// `sstatus`, traps on `mstatus`, and returns into user mode. User mode
/// reads `instret`, and traps on `cycle`, `wfi` and `sret`, each to the
/// supervisor's handler, which steps past it; its `ecall` goes to the
/// supervisor too, which calls machine mode in turn, and machine mode,
/// which delegated nothing from supervisor mode, halts.
#[test]
fn user_and_supervisor_modes_trap_where_they_are_sent() {
    use vreteno32::isa::*;
    use vreteno32::program::Asm;
    let mut a = Asm::default();
    let (mh, sh, s_code, u_code, skip) =
        (a.label(), a.label(), a.label(), a.label(), a.label());
    a.wide(addi(8, 0, 0)); // x8 counts the supervisor's traps
    a.abs(mh, |h| addi(31, 0, h as i32));
    a.wide(csrrw(0, CSR_MTVEC, 31));
    a.abs(sh, |h| addi(31, 0, h as i32));
    a.wide(csrrw(0, CSR_STVEC, 31));
    a.wide(addi(5, 0, 0x104)); // ecall from U and an illegal instruction
    a.wide(csrrw(0, CSR_MEDELEG, 5));
    a.wide(csrrwi(0, CSR_MCOUNTEREN, 4)); // instret, not cycle
                                          // mstatush: a write is ignored and a read is zero (issue 1076).
    a.wide(addi(12, 0, -1));
    a.wide(csrrc(0, CSR_MSTATUSH, 12));
    a.wide(csrrw(13, CSR_MSTATUSH, 12));
    a.wide(csrrs(14, CSR_MSTATUSH, 0));
    a.wide(csrrwi(0, CSR_SCOUNTEREN, 4));
    a.wide(lui(5, 1));
    a.wide(addi(5, 5, -0x800)); // x5 = 0x800, MPP = supervisor
    a.wide(csrrw(0, CSR_MSTATUS, 5));
    a.abs(s_code, |s| addi(31, 0, s as i32));
    a.wide(csrrw(0, CSR_MEPC, 31));
    a.wide(mret());
    // Supervisor mode.
    a.place(s_code);
    a.wide(csrrs(6, CSR_SSTATUS, 0));
    a.wide(csrrs(7, CSR_MSTATUS, 0)); // illegal here
    a.wide(addi(9, 0, 0x100));
    a.wide(csrrc(0, CSR_SSTATUS, 9)); // SPP = user
    a.abs(u_code, |u| addi(31, 0, u as i32));
    a.wide(csrrw(0, CSR_SEPC, 31));
    a.wide(sret());
    // User mode.
    a.place(u_code);
    // Into x0: a counter's value is the pipeline's and not the
    // model's to know, so only that the read is legal is checked.
    a.wide(csrrs(0, CSR_INSTRET, 0));
    a.wide(csrrs(11, CSR_CYCLE, 0)); // not enabled
    a.wide(wfi()); // illegal in user mode
    a.wide(sret()); // and so is this
    a.wide(ecall());
    a.wide(halt()); // never reached
                    // The supervisor's handler: an environment call from user mode is
                    // passed on to machine mode; anything else is stepped past.
    a.align();
    a.place(sh);
    a.wide(csrrs(22, CSR_SCAUSE, 0));
    a.wide(addi(8, 8, 1));
    a.wide(addi(23, 0, 8));
    a.to(skip, |o| bne(22, 23, o));
    a.wide(ecall());
    a.place(skip);
    a.wide(csrrs(24, CSR_SEPC, 0));
    a.wide(addi(24, 24, 4));
    a.wide(csrrw(0, CSR_SEPC, 24));
    a.wide(sret());
    // Machine mode's handler: the cause, the mode it came from, done.
    a.align();
    a.place(mh);
    a.wide(csrrs(21, CSR_MCAUSE, 0));
    a.wide(csrrs(25, CSR_MSTATUS, 0));
    a.wide(halt());
    let m = lockstep(&a.words(), &[], "modes", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[8], 5, "mstatus, cycle, wfi, sret and the ecall");
    assert_eq!(m.x[22], 8, "the last the supervisor saw: ecall from U");
    assert_eq!(m.x[21], 9, "machine mode saw the supervisor's ecall");
    assert_eq!(m.x[25] >> 11 & 3, 1, "MPP says supervisor");
    assert_eq!(m.x[7], 0, "the illegal read wrote nothing");
    assert_eq!(m.x[11], 0, "nor the disabled counter");
    assert_eq!(m.prv, 3, "ending in machine mode");
    assert_eq!([m.x[13], m.x[14]], [0, 0], "mstatush holds nothing");
}

/// mstatus.TVM, TW and TSR (issue 1347): machine mode sets all three
/// with MPP, reads mstatus back, reads satp itself, which TVM leaves
/// legal in machine mode, and returns into supervisor mode. There a
/// read of satp, sfence.vma, wfi and sret are each an illegal
/// instruction, taken to machine mode, which counts it and steps past
/// it; the supervisor's ecall ends the run.
#[test]
fn tvm_tw_and_tsr_trap_the_supervisor() {
    use vreteno32::isa::*;
    use vreteno32::program::Asm;
    let mut a = Asm::default();
    let (mh, s_code, done) = (a.label(), a.label(), a.label());
    a.wide(addi(8, 0, 0)); // x8 counts the illegal instructions
    a.abs(mh, |h| addi(31, 0, h as i32));
    a.wide(csrrw(0, CSR_MTVEC, 31));
    a.wide(lui(5, 0x700)); // TVM, TW and TSR
    a.wide(addi(5, 5, 0x400));
    a.wide(addi(5, 5, 0x400)); // and MPP = supervisor
    a.wide(csrrw(0, CSR_MSTATUS, 5));
    a.wide(csrrs(10, CSR_MSTATUS, 0)); // 0x0070_0800
    a.wide(csrrs(11, CSR_SATP, 0)); // legal in machine mode
    a.abs(s_code, |c| addi(31, 0, c as i32));
    a.wide(csrrw(0, CSR_MEPC, 31));
    a.wide(mret());
    // Supervisor mode.
    a.place(s_code);
    a.wide(csrrs(12, CSR_SATP, 0)); // TVM
    a.wide(sfence_vma(0, 0)); // TVM
    a.wide(wfi()); // TW
    a.wide(sret()); // TSR
    a.wide(ecall());
    a.wide(halt()); // never reached
                    // Machine mode's handler: an illegal instruction is counted and
                    // stepped past, back into supervisor mode; the ecall ends it.
    a.align();
    a.place(mh);
    a.wide(csrrs(21, CSR_MCAUSE, 0));
    a.wide(addi(22, 0, 2));
    a.to(done, |o| bne(21, 22, o));
    a.wide(addi(8, 8, 1));
    a.wide(csrrs(24, CSR_MEPC, 0));
    a.wide(addi(24, 24, 4));
    a.wide(csrrw(0, CSR_MEPC, 24));
    a.wide(mret());
    a.place(done);
    a.wide(halt());
    let m = lockstep(&a.words(), &[], "tvm tw tsr", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[10], 0x0070_0800, "TVM, TW, TSR and MPP read back");
    assert_eq!(m.x[8], 4, "satp, sfence.vma, wfi and sret trapped");
    assert_eq!(m.x[21], 9, "and the supervisor's ecall ended it");
    assert_eq!(m.x[12], 0, "the trapped read wrote nothing");
}

/// menvcfg, menvcfgh and senvcfg (issue 1348): each takes a write of
/// all ones and keeps FIOM alone, menvcfgh nothing; supervisor mode
/// reads senvcfg and not menvcfg, which is an illegal instruction there.
#[test]
fn the_envcfg_registers_keep_fiom_alone() {
    use vreteno32::isa::*;
    use vreteno32::program::Asm;
    let mut a = Asm::default();
    let (mh, s_code, done) = (a.label(), a.label(), a.label());
    a.wide(addi(8, 0, 0)); // x8 counts the illegal instructions
    a.abs(mh, |h| addi(31, 0, h as i32));
    a.wide(csrrw(0, CSR_MTVEC, 31));
    a.wide(addi(5, 0, -1));
    a.wide(csrrw(0, CSR_MENVCFG, 5));
    a.wide(csrrs(10, CSR_MENVCFG, 0)); // 1
    a.wide(csrrw(0, CSR_MENVCFGH, 5));
    a.wide(csrrs(11, CSR_MENVCFGH, 0)); // 0
    a.wide(csrrw(0, CSR_SENVCFG, 5));
    a.wide(csrrs(12, CSR_SENVCFG, 0)); // 1
    a.wide(lui(5, 1));
    a.wide(addi(5, 5, -0x800)); // MPP = supervisor
    a.wide(csrrw(0, CSR_MSTATUS, 5));
    a.abs(s_code, |c| addi(31, 0, c as i32));
    a.wide(csrrw(0, CSR_MEPC, 31));
    a.wide(mret());
    // Supervisor mode.
    a.place(s_code);
    a.wide(csrrs(13, CSR_SENVCFG, 0)); // 1
    a.wide(csrrs(14, CSR_MENVCFG, 0)); // illegal here
    a.wide(ecall());
    a.wide(halt()); // never reached
                    // Machine mode's handler: an illegal instruction is counted and
                    // stepped past; the ecall ends it.
    a.align();
    a.place(mh);
    a.wide(csrrs(21, CSR_MCAUSE, 0));
    a.wide(addi(22, 0, 2));
    a.to(done, |o| bne(21, 22, o));
    a.wide(addi(8, 8, 1));
    a.wide(csrrs(24, CSR_MEPC, 0));
    a.wide(addi(24, 24, 4));
    a.wide(csrrw(0, CSR_MEPC, 24));
    a.wide(mret());
    a.place(done);
    a.wide(halt());
    let m = lockstep(&a.words(), &[], "envcfg", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[10], 1, "menvcfg keeps FIOM");
    assert_eq!(m.x[11], 0, "menvcfgh keeps nothing");
    assert_eq!(m.x[12], 1, "senvcfg keeps FIOM");
    assert_eq!(m.x[13], 1, "supervisor mode reads senvcfg");
    assert_eq!(m.x[8], 1, "and not menvcfg");
    assert_eq!(m.x[14], 0, "which wrote nothing");
    assert_eq!(m.x[21], 9, "the supervisor's ecall ended it");
}

/// A delegated interrupt (issue 1012): machine mode delegates the
/// supervisor's software interrupt, enables it, raises it in `mip` and
/// returns into supervisor mode with `SIE` set; the interrupt is taken
/// there at once, by the supervisor's handler, which clears it through
/// `sip` and returns to the instruction it interrupted.
#[test]
fn a_delegated_interrupt_reaches_the_supervisor() {
    use vreteno32::isa::*;
    use vreteno32::program::Asm;
    let mut a = Asm::default();
    let (mh, sh, s_code) = (a.label(), a.label(), a.label());
    a.wide(addi(8, 0, 0));
    a.abs(mh, |h| addi(31, 0, h as i32));
    a.wide(csrrw(0, CSR_MTVEC, 31));
    a.abs(sh, |h| addi(31, 0, h as i32));
    a.wide(csrrw(0, CSR_STVEC, 31));
    a.wide(csrrwi(0, CSR_MIDELEG, 2));
    a.wide(csrrwi(0, CSR_MIE, 2));
    a.wide(lui(5, 1));
    a.wide(addi(5, 5, -0x800 + 2)); // MPP = supervisor, SIE
    a.wide(csrrw(0, CSR_MSTATUS, 5));
    a.abs(s_code, |s| addi(31, 0, s as i32));
    a.wide(csrrw(0, CSR_MEPC, 31));
    a.wide(csrrsi(0, CSR_MIP, 2)); // the supervisor's software interrupt
    a.wide(mret());
    a.place(s_code);
    a.wide(addi(9, 0, 7));
    a.wide(ecall());
    a.align();
    a.place(sh);
    a.wide(csrrs(22, CSR_SCAUSE, 0));
    a.wide(addi(8, 8, 1));
    a.wide(csrrci(0, CSR_SIP, 2));
    a.wide(sret());
    a.align();
    a.place(mh);
    a.wide(csrrs(21, CSR_MCAUSE, 0));
    a.wide(halt());
    let m = lockstep(&a.words(), &[], "delegated", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[8], 1, "taken once");
    assert_eq!(m.x[22], CAUSE_SSOFT, "as the supervisor's software one");
    assert_eq!(m.x[9], 7, "and the interrupted code ran after");
    assert_eq!(m.x[21], CAUSE_ECALL_S, "then called machine mode");
}

/// The interrupt controller's supervisor line reaches `mip.SEIP`
/// (issue 1094). In machine mode, with the line high, a clear of
/// another bit of `mip` leaves the software's SEIP alone, so the bit
/// falls with the line. Then, delegated, the line's next pulse is taken
/// by the supervisor's handler as its external interrupt.
#[test]
fn the_supervisor_external_line_reaches_the_supervisor() {
    use vreteno32::isa::*;
    use vreteno32::program::Asm;
    let mut a = Asm::default();
    let (mh, sh, s_code) = (a.label(), a.label(), a.label());
    let (wait_hi, wait_lo, s_loop, s_wait) =
        (a.label(), a.label(), a.label(), a.label());
    a.wide(addi(8, 0, 0));
    a.abs(mh, |h| addi(31, 0, h as i32));
    a.wide(csrrw(0, CSR_MTVEC, 31));
    a.abs(sh, |h| addi(31, 0, h as i32));
    a.wide(csrrw(0, CSR_STVEC, 31));
    a.wide(addi(5, 0, SEXT as i32));
    a.wide(csrrw(0, CSR_MIDELEG, 5));
    a.wide(csrrw(0, CSR_MIE, 5));
    // The first pulse, in machine mode: a clear of SSIP while the line
    // is high must not keep SEIP once the line falls.
    a.place(wait_hi);
    a.wide(csrrs(6, CSR_MIP, 0));
    a.wide(andi(6, 6, SEXT as i32));
    a.to(wait_hi, |o| beq(6, 0, o));
    a.wide(csrrci(0, CSR_MIP, 2));
    a.place(wait_lo);
    a.wide(csrrs(6, CSR_MIP, 0));
    a.wide(andi(6, 6, SEXT as i32));
    a.to(wait_lo, |o| bne(6, 0, o));
    a.wide(csrrs(23, CSR_MIP, 0));
    // Down to supervisor mode with SIE, to wait for the second pulse.
    a.wide(lui(5, 1));
    a.wide(addi(5, 5, -0x800 + 2)); // MPP = supervisor, SIE
    a.wide(csrrw(0, CSR_MSTATUS, 5));
    a.abs(s_code, |s| addi(31, 0, s as i32));
    a.wide(csrrw(0, CSR_MEPC, 31));
    a.wide(mret());
    a.place(s_code);
    a.place(s_loop);
    a.wide(addi(9, 9, 1));
    a.to(s_loop, |o| beq(8, 0, o));
    a.wide(ecall());
    // The supervisor's handler: the cause, a count, and a wait for the
    // line to fall, since only the controller can lower it.
    a.align();
    a.place(sh);
    a.wide(csrrs(22, CSR_SCAUSE, 0));
    a.wide(addi(8, 8, 1));
    a.place(s_wait);
    a.wide(csrrs(6, CSR_SIP, 0));
    a.wide(andi(6, 6, SEXT as i32));
    a.to(s_wait, |o| bne(6, 0, o));
    a.wide(sret());
    a.align();
    a.place(mh);
    a.wide(csrrs(21, CSR_MCAUSE, 0));
    a.wide(halt());
    let m = lockstep_with(
        &a.words(),
        &[],
        "seip",
        None,
        None,
        None,
        Some(|c| (300..340).contains(&c) || (900..940).contains(&c)),
    );
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[23] & SEXT, 0, "SEIP fell with the line");
    assert_eq!(m.x[8], 1, "taken once, in supervisor mode");
    assert_eq!(m.x[22], CAUSE_SEXT, "as the supervisor's external one");
    assert!(m.x[9] > 0, "the supervisor's code ran before it");
    assert_eq!(m.x[21], CAUSE_ECALL_S, "then called machine mode");
}

/// Two instructions that load `v` into `rd`.
fn li(a: &mut vreteno32::program::Asm, rd: u32, v: u32) {
    use vreteno32::isa::{addi, lui};
    a.wide(lui(rd, v.wrapping_add(0x800) >> 12));
    a.wide(addi(rd, rd, ((v << 20) as i32) >> 20));
}

/// Two instructions that load `base` plus a label's address into `rd`.
fn la(a: &mut vreteno32::program::Asm, rd: u32, l: usize, base: u32) {
    use vreteno32::isa::{addi, lui};
    a.abs(l, move |x| {
        lui(rd, base.wrapping_add(x).wrapping_add(0x800) >> 12)
    });
    a.abs(l, move |x| {
        addi(rd, rd, ((base.wrapping_add(x) << 20) as i32) >> 20)
    });
}

/// Virtual memory (issue 1014). Machine mode writes page tables into
/// the data memory, turns translation on in `satp`, delegates the load
/// and the store page faults, and returns into supervisor mode at a
/// virtual address; the supervisor then loads, stores and runs atomics
/// through pages that allow them and pages that do not, and jumps
/// where nothing is mapped.
///
/// The data memory is one page, so the root table and the second level
/// are that page both, and their entries sit at indices nothing else
/// uses: the root's at 0x200, 0x300 and 0x301, the second level's from
/// 0x210, and the program's data below 0x800. The code runs from the
/// boot memory, mapped as a megapage at `0x8000_0000`, and once as a
/// page whose next page is unmapped, where an instruction whose second
/// half is on that next page faults there.
#[test]
fn sv32_translates_and_faults_where_the_tables_say() {
    use txhdl_parts::mmu::pte::{to, A, D, R, U, V, W, X};
    use vreteno32::isa::*;
    use vreteno32::program::Asm;
    const CODE: u32 = 0x8000_0000;
    const DATA: u32 = 0xc021_0000;
    let page = |j: u32| 0xc000_0000 | (j << 12);
    let mut a = Asm::default();
    let (mh, sh, s_code, not_ecall, skip) =
        (a.label(), a.label(), a.label(), a.label(), a.label());
    // The tables, an entry at a time, from x3 = 0x1800, index 0x200.
    a.wide(lui(3, 2));
    a.wide(addi(3, 3, -0x800));
    let entry = |a: &mut Asm, idx: u32, v: u32| {
        li(a, 5, v);
        a.wide(sw(5, 3, (4 * idx) as i32 - 0x800));
    };
    entry(&mut a, 0x200, to(0, V | R | X | A));
    entry(&mut a, 0x300, to(0x1000, V));
    entry(&mut a, 0x301, to(0x3000_0000, V));
    entry(&mut a, 0x210, to(0x1000, V | R | W | A | D));
    entry(&mut a, 0x211, to(0x1000, V | R | A));
    entry(&mut a, 0x212, to(0x1000, V | R | W | A));
    entry(&mut a, 0x213, to(0x1000, V | R | W | U | A | D));
    entry(&mut a, 0x215, to(0x1000, V | X | A));
    entry(&mut a, 0x216, to(0x3000_0000, V | R | W | A | D));
    entry(&mut a, 0x217, to(0x1000, V | W | A | D));
    entry(&mut a, 0x218, to(0x1000, V));
    entry(&mut a, 0x222, to(0, V | X | A));
    for i in 0..10 {
        entry(&mut a, 0x230 + i, to(0x1000, V | R | W | A | D));
    }
    li(&mut a, 5, txhdl_parts::mmu::satp(0x1000));
    a.wide(csrrw(0, CSR_SATP, 5));
    la(&mut a, 31, mh, 0);
    a.wide(csrrw(0, CSR_MTVEC, 31));
    la(&mut a, 31, sh, CODE);
    a.wide(csrrw(0, CSR_STVEC, 31));
    li(&mut a, 5, 1 << 13 | 1 << 15);
    a.wide(csrrw(0, CSR_MEDELEG, 5));
    li(&mut a, 5, 0x800); // MPP = supervisor
    a.wide(csrrw(0, CSR_MSTATUS, 5));
    li(&mut a, 30, DATA + 0x100); // the supervisor's log, virtual
    li(&mut a, 28, 0x1400); // the machine's, physical
    a.wide(addi(8, 0, 0));
    a.wide(addi(9, 0, 0));
    la(&mut a, 31, s_code, CODE);
    a.wide(csrrw(0, CSR_MEPC, 31));
    a.wide(mret());

    // Supervisor mode, at a virtual address.
    a.place(s_code);
    li(&mut a, 10, DATA);
    a.wide(addi(11, 0, 0x55));
    a.wide(sw(11, 10, 0x10));
    a.wide(lw(12, 10, 0x10));
    li(&mut a, 13, page(0x211)); // read only
    a.wide(lw(14, 13, 0x10));
    a.wide(sw(11, 13, 0x10));
    li(&mut a, 13, page(0x212)); // dirty bit clear
    a.wide(lw(14, 13, 0x10));
    a.wide(sw(11, 13, 0x10));
    li(&mut a, 13, page(0x213)); // a user page, then with SUM
    a.wide(lw(15, 13, 0x10));
    li(&mut a, 5, 1 << 18);
    a.wide(csrrs(0, CSR_SSTATUS, 5));
    a.wide(lw(15, 13, 0x10));
    a.wide(csrrc(0, CSR_SSTATUS, 5));
    li(&mut a, 13, page(0x214)); // invalid
    a.wide(lw(16, 13, 0x10));
    a.wide(sw(11, 13, 0x10));
    a.wide(amoadd_w(17, 13, 11));
    a.wide(lr_w(17, 13));
    a.wide(lw(16, 13, 0x11)); // misaligned there: that, not the page
    li(&mut a, 13, page(0x215)); // execute only, then with MXR
    a.wide(lw(16, 13, 0x10));
    li(&mut a, 5, 1 << 19);
    a.wide(csrrs(0, CSR_SSTATUS, 5));
    a.wide(lw(16, 13, 0x10));
    a.wide(csrrc(0, CSR_SSTATUS, 5));
    li(&mut a, 13, page(0x216)); // where nothing answers
    a.wide(lw(16, 13, 0x10));
    li(&mut a, 13, page(0x217)); // write without read
    a.wide(lw(16, 13, 0x10));
    li(&mut a, 13, page(0x218)); // a pointer at the last level
    a.wide(lw(16, 13, 0x10));
    li(&mut a, 13, 0xc040_0000); // a table where nothing answers
    a.wide(lw(16, 13, 0));
    li(&mut a, 13, DATA + 0x20); // atomics where they may
    a.wide(amoadd_w(18, 13, 11));
    a.wide(lr_w(19, 13));
    a.wide(sc_w(20, 13, 11));
    li(&mut a, 13, page(0x211) + 0x20); // and where they may only read
    a.wide(lr_w(19, 13));
    a.wide(sc_w(20, 13, 11));
    a.wide(amoswap_w(18, 13, 11));
    // Ten pages, twice, past the eight entries the data's buffer has.
    for _ in 0..2 {
        for i in 0..10 {
            li(&mut a, 13, page(0x230 + i));
            a.wide(addi(11, 11, 1));
            a.wide(sw(11, 13, 0x30));
            a.wide(lw(6, 13, 0x30));
        }
    }
    // Take the write away from the first of them through its own
    // entry, then drop the translations: it reads and does not write.
    li(&mut a, 13, DATA + 4 * 0x230);
    li(&mut a, 5, to(0x1000, V | R | A));
    a.wide(sw(5, 13, 0));
    a.wide(sfence_vma(0, 0));
    li(&mut a, 13, page(0x230));
    a.wide(lw(6, 13, 0x30));
    a.wide(sw(6, 13, 0x30));
    // A jump where no page is, and one to an instruction whose second
    // half is on a page that is not there.
    li(&mut a, 13, 0x8040_0000);
    a.wide(jalr(1, 13, 0));
    li(&mut a, 13, page(0x222) + 0xffe);
    a.wide(jalr(1, 13, 0));
    a.wide(ecall());

    // The supervisor's handler: log the cause and the value, step past.
    a.align();
    a.place(sh);
    a.wide(csrrs(24, CSR_SCAUSE, 0));
    a.wide(csrrs(25, CSR_STVAL, 0));
    a.wide(sw(24, 30, 0));
    a.wide(sw(25, 30, 4));
    a.wide(addi(30, 30, 8));
    a.wide(addi(8, 8, 1));
    a.wide(csrrs(26, CSR_SEPC, 0));
    a.wide(addi(26, 26, 4));
    a.wide(csrrw(0, CSR_SEPC, 26));
    a.wide(sret());
    // Machine mode's: log, halt on the supervisor's call, return from a
    // fetch's fault to where the jump came from, and else step past.
    a.align();
    a.place(mh);
    a.wide(csrrs(21, CSR_MCAUSE, 0));
    a.wide(csrrs(22, CSR_MTVAL, 0));
    a.wide(sw(21, 28, 0));
    a.wide(sw(22, 28, 4));
    a.wide(addi(28, 28, 8));
    a.wide(addi(9, 9, 1));
    a.wide(addi(23, 0, 9));
    a.to(not_ecall, |o| bne(21, 23, o));
    a.wide(halt());
    a.place(not_ecall);
    a.wide(addi(23, 0, 12));
    a.to(skip, |o| bne(21, 23, o));
    a.wide(csrrw(0, CSR_MEPC, 1));
    a.wide(mret());
    a.place(skip);
    a.wide(csrrs(23, CSR_MEPC, 0));
    a.wide(addi(23, 23, 4));
    a.wide(csrrw(0, CSR_MEPC, 23));
    a.wide(mret());
    // The first half of a thirty-two bit instruction in the boot
    // memory's last halfword.
    assert!(a.here() <= 0xffe, "the program is {} bytes", a.here());
    a.halves.resize(0x7ff, 0);
    a.emit_c(0x0013);
    let m = lockstep(&a.words(), &[], "sv32", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    let log = |at: u32, n: usize| -> Vec<(u32, u32)> {
        let w = (at - vreteno32::model::DATA_BASE) as usize / 4;
        (0..n)
            .map(|i| (m.mem[w + 2 * i], m.mem[w + 2 * i + 1]))
            .collect()
    };
    let (lp, sp) = (CAUSE_LOAD_PAGE, CAUSE_STORE_PAGE);
    assert_eq!(
        log(0x1100, 13),
        vec![
            (sp, page(0x211) + 0x10),
            (sp, page(0x212) + 0x10),
            (lp, page(0x213) + 0x10),
            (lp, page(0x214) + 0x10),
            (sp, page(0x214) + 0x10),
            (sp, page(0x214)),
            (lp, page(0x214)),
            (lp, page(0x215) + 0x10),
            (lp, page(0x217) + 0x10),
            (lp, page(0x218) + 0x10),
            (sp, page(0x211) + 0x20),
            (sp, page(0x211) + 0x20),
            (sp, page(0x230) + 0x30),
        ],
        "what the supervisor was sent"
    );
    assert_eq!(m.x[8], 13);
    assert_eq!(
        log(0x1400, 6),
        vec![
            (CAUSE_LOAD_MISALIGNED, page(0x214) + 0x11),
            (CAUSE_LOAD_ACCESS, page(0x216) + 0x10),
            (CAUSE_LOAD_ACCESS, 0xc040_0000),
            (CAUSE_FETCH_PAGE, 0x8040_0000),
            (CAUSE_FETCH_PAGE, page(0x223)),
            (CAUSE_ECALL_S, 0),
        ],
        "what machine mode was sent"
    );
    assert_eq!(m.x[12], 0x55, "a load through a page");
    assert_eq!(m.x[20], 0, "sc.w stored where it may");
}

/// A load that traps in execute traps once, for its own cause, whatever
/// the bus said about the load before it: a misaligned load right after
/// a refused one is a misaligned load and nothing more.
#[test]
fn a_load_that_traps_in_execute_is_not_refused_as_well() {
    let handler = 8 * 4;
    let p = [
        addi(6, 0, handler),
        csrrw(0, CSR_MTVEC, 6),
        lui(4, 0x3000), // 0x0300_0000: nobody's
        lw(5, 4, 0),    // refused: a load access fault
        lw(5, 4, 1),    // misaligned, and only that
        addi(7, 0, 1),
        halt(),
        addi(0, 0, 0),
        // The handler, at 32: counts, keeps the causes in order a
        // nibble each, and steps past.
        csrrs(23, CSR_MCAUSE, 0),
        addi(25, 25, 1),
        vreteno32::isa::slli(28, 28, 4),
        add(28, 28, 23),
        csrrs(27, CSR_MEPC, 0),
        addi(27, 27, 4),
        csrrw(0, CSR_MEPC, 27),
        mret(),
    ];
    let m = lockstep(&p, &[], "a refusal, then a trap", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[28], 0x54, "an access fault, then a misaligned load");
    assert_eq!(m.x[25], 2, "two traps");
}

/// Random programs under Sv32 (issue 1014), over page tables the seed
/// chooses, in supervisor and in user mode: every page fault and its
/// handling, the delegated ones and machine mode's, with a fix and a
/// flush now and then, against the model, which keeps no translations.
#[test]
fn random_programs_under_paging() {
    use vreteno32::isa::{CAUSE_ILLEGAL, CAUSE_LOAD_PAGE, CAUSE_STORE_PAGE};
    let mut causes = [0u32; 16];
    for seed in 0..96 {
        let p = vreteno32::program::random_vm(seed, 300);
        let m = lockstep(
            &p,
            &[],
            &format!("random paged seed {seed}"),
            Some(seed),
            None,
            None,
        );
        assert_eq!(m.halted, Some(Halt::Break), "seed {seed} faulted");
        for (c, n) in causes.iter_mut().zip(m.causes) {
            *c += n;
        }
    }
    let seen = format!("exceptions by cause: {causes:?}");
    assert!(causes[CAUSE_LOAD_PAGE as usize] >= 50, "{seen}");
    assert!(causes[CAUSE_STORE_PAGE as usize] >= 50, "{seen}");
    assert!(causes[CAUSE_ILLEGAL as usize] >= 50, "{seen}");
}

/// `fence.i` makes what a program stored into code what it then runs:
/// a word run from the data memory, rewritten, and run again after the
/// fence runs as rewritten, though the fetch's buffer still held it.
/// The code at 0x1100 is `addi x5, x5, 1` and a return, and two returns
/// after that. While the first return runs, the fetch takes the word
/// after it, at 0x1108, which is the word its buffer then holds; that
/// word is rewritten into `addi x5, x5, 2` and jumped to.
#[test]
fn fence_i_runs_the_code_as_stored() {
    use vreteno32::isa::*;
    // The new word, addi x5, x5, 2, in two instructions.
    let w = addi(5, 5, 2);
    let p = [
        lui(2, 1),         // the data memory, where the code is
        addi(5, 0, 0),     // x5 counts what the code added
        jalr(1, 2, 0x100), // run it: x5 += 1
        lui(6, w.wrapping_add(0x800) >> 12),
        addi(6, 6, ((w << 20) as i32) >> 20),
        sw(6, 2, 0x108),
        0x0000_100f,       // fence.i
        jalr(1, 2, 0x108), // run the rewritten word: x5 += 2
        halt(),
    ];
    let mut data = vec![0u32; 0x48];
    data[0x40] = addi(5, 5, 1);
    data[0x41] = jalr(0, 1, 0);
    data[0x42] = jalr(0, 1, 0);
    data[0x43] = jalr(0, 1, 0);
    let m = lockstep(&p, &data, "fence.i", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[5], 3, "once as it was, once as rewritten");
}

/// `MPRV` (issue 1105): machine mode, with translation on and `MPRV`
/// set, loads and stores as the mode `MPP` names, which is how OpenSBI
/// reads the instruction a supervisor trapped on. A supervisor's page
/// reads and writes; a user page faults until `SUM` is set; the fetch
/// stays machine mode's, untranslated; a trap taken meanwhile sets
/// `MPP` to machine mode, so the handler's own accesses are physical;
/// and a return to supervisor mode clears `MPRV`.
#[test]
fn mprv_loads_and_stores_as_the_previous_mode() {
    use txhdl_parts::mmu::pte::{to, A, D, R, U, V, W, X};
    use vreteno32::isa::*;
    use vreteno32::program::Asm;
    let page = |j: u32| 0xc000_0000 | (j << 12);
    let mut a = Asm::default();
    let (mh, s_code, not_ecall) = (a.label(), a.label(), a.label());
    // The tables, from x3 = 0x1800: the boot memory as a supervisor's
    // megapage where it is, and two pages of the second level over the
    // data memory, a supervisor's and a user's.
    a.wide(lui(3, 2));
    a.wide(addi(3, 3, -0x800));
    let entry = |a: &mut Asm, idx: u32, v: u32| {
        li(a, 5, v);
        a.wide(sw(5, 3, (4 * idx) as i32 - 0x800));
    };
    entry(&mut a, 0x200, to(0, V | R | X | A));
    entry(&mut a, 0x300, to(0x1000, V));
    entry(&mut a, 0x210, to(0x1000, V | R | W | A | D));
    entry(&mut a, 0x213, to(0x1000, V | R | W | U | A | D));
    // A word to read, at 0x1010, written physically.
    a.wide(lui(9, 1));
    li(&mut a, 7, 0x1234);
    a.wide(sw(7, 9, 0x10));
    la(&mut a, 31, mh, 0);
    a.wide(csrrw(0, CSR_MTVEC, 31));
    li(&mut a, 5, txhdl_parts::mmu::satp(0x1000));
    a.wide(csrrw(0, CSR_SATP, 5));
    li(&mut a, 28, 0x1400); // machine mode's log, physical
                            // MPRV, with MPP the supervisor's.
    li(&mut a, 5, 1 << 17 | 1 << 11);
    a.wide(csrrs(0, CSR_MSTATUS, 5));
    li(&mut a, 10, page(0x210));
    a.wide(lw(11, 10, 0x10)); // 0x1234, through the table
    a.wide(addi(12, 11, 1));
    a.wide(sw(12, 10, 0x14)); // to 0x1014
    li(&mut a, 13, page(0x213));
    // A user page: faults, and is stepped past.
    a.wide(lw(14, 13, 0x10));
    // The handler's return left MPP at user mode: the supervisor's
    // again, and SUM, and the user page reads.
    li(&mut a, 5, 1 << 11 | 1 << 18);
    a.wide(csrrs(0, CSR_MSTATUS, 5));
    a.wide(lw(15, 13, 0x10)); // 0x1234
                              // Into supervisor mode, which clears MPRV.
    la(&mut a, 31, s_code, 0x8000_0000);
    a.wide(csrrw(0, CSR_MEPC, 31));
    a.wide(mret());
    a.place(s_code);
    a.wide(lw(16, 10, 0x14)); // 0x1235, what machine mode stored
    a.wide(ecall());
    // Machine mode's handler: log, and halt on the supervisor's call
    // with mstatus in x20, else step past.
    a.align();
    a.place(mh);
    a.wide(csrrs(21, CSR_MCAUSE, 0));
    a.wide(csrrs(22, CSR_MTVAL, 0));
    a.wide(sw(21, 28, 0));
    a.wide(sw(22, 28, 4));
    a.wide(addi(28, 28, 8));
    a.wide(addi(23, 0, 9));
    a.to(not_ecall, |o| bne(21, 23, o));
    a.wide(csrrs(20, CSR_MSTATUS, 0));
    a.wide(halt());
    a.place(not_ecall);
    a.wide(csrrs(23, CSR_MEPC, 0));
    a.wide(addi(23, 23, 4));
    a.wide(csrrw(0, CSR_MEPC, 23));
    a.wide(mret());
    let m = lockstep(&a.words(), &[], "mprv", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[11], 0x1234, "machine mode read through the table");
    assert_eq!(m.mem[5], 0x1235, "and wrote through it");
    assert_eq!(m.x[14], 0, "the user page faulted and wrote nothing");
    assert_eq!(m.x[15], 0x1234, "and read under SUM");
    assert_eq!(m.x[16], 0x1235, "the supervisor reads what it wrote");
    let log = (m.mem[0x100], m.mem[0x101], m.mem[0x102]);
    assert_eq!(log, (CAUSE_LOAD_PAGE, page(0x213) + 0x10, CAUSE_ECALL_S));
    assert_eq!(m.x[20] >> 17 & 1, 0, "the return cleared MPRV");
}

/// A misaligned access goes where `medeleg` sends its cause, which the
/// core chooses last, by the misaligned check (issue 1130): a load's,
/// delegated, to the supervisor's handler, and a store's, not, to
/// machine mode's, each from supervisor mode.
#[test]
fn a_misaligned_access_traps_where_it_is_delegated() {
    use vreteno32::isa::*;
    use vreteno32::program::Asm;
    let mut a = Asm::default();
    let (mh, sh, s_code, not_ecall) =
        (a.label(), a.label(), a.label(), a.label());
    a.abs(mh, |h| addi(31, 0, h as i32));
    a.wide(csrrw(0, CSR_MTVEC, 31));
    a.abs(sh, |h| addi(31, 0, h as i32));
    a.wide(csrrw(0, CSR_STVEC, 31));
    a.wide(addi(5, 0, 1 << 4)); // a misaligned load, not a store
    a.wide(csrrw(0, CSR_MEDELEG, 5));
    a.wide(lui(5, 1));
    a.wide(addi(5, 5, -0x800)); // MPP = supervisor
    a.wide(csrrw(0, CSR_MSTATUS, 5));
    a.abs(s_code, |s| addi(31, 0, s as i32));
    a.wide(csrrw(0, CSR_MEPC, 31));
    a.wide(mret());
    a.place(s_code);
    a.wide(lui(2, 1)); // the data memory
    a.wide(lw(10, 2, 2)); // to the supervisor
    a.wide(sw(10, 2, 1)); // to machine mode
    a.wide(ecall());
    a.align();
    a.place(sh);
    a.wide(csrrs(22, CSR_SCAUSE, 0));
    a.wide(csrrs(23, CSR_STVAL, 0));
    a.wide(csrrs(24, CSR_SEPC, 0));
    a.wide(addi(24, 24, 4));
    a.wide(csrrw(0, CSR_SEPC, 24));
    a.wide(sret());
    a.align();
    a.place(mh);
    a.wide(csrrs(21, CSR_MCAUSE, 0));
    a.wide(addi(26, 0, 9));
    a.to(not_ecall, |o| bne(21, 26, o));
    a.wide(halt());
    a.place(not_ecall);
    a.wide(addi(20, 21, 0));
    a.wide(csrrs(25, CSR_MTVAL, 0));
    a.wide(csrrs(26, CSR_MEPC, 0));
    a.wide(addi(26, 26, 4));
    a.wide(csrrw(0, CSR_MEPC, 26));
    a.wide(mret());
    let m =
        lockstep(&a.words(), &[], "misaligned, delegated", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!((m.x[22], m.x[23]), (CAUSE_LOAD_MISALIGNED, 0x1002));
    assert_eq!((m.x[20], m.x[25]), (CAUSE_STORE_MISALIGNED, 0x1001));
}

/// A branch guessed wrong, then a trap at once on the right path (issue
/// 1300): a forward branch taken, which was guessed not taken, lands on
/// an `ecall`; a backward branch not taken at the end of its loop,
/// which was guessed taken, falls on another. The handler counts both
/// and steps past them, and the word skipped is never run.
#[test]
fn a_branch_guessed_wrong_then_a_trap() {
    use vreteno32::isa::*;
    const HANDLER: u32 = 0x80;
    let mut p = vec![
        addi(5, 0, HANDLER as i32),
        csrrw(0, CSR_MTVEC, 5),
        beq(0, 0, 8),    // forward, taken: guessed wrong
        addi(1, 1, 100), // skipped
        ecall(),         // the trap right after the correction
        addi(6, 0, 3),
        addi(6, 6, -1), // the loop
        bne(6, 0, -4),  // backward: taken twice, then not, wrongly
        ecall(),        // the trap right after that correction
        halt(),
    ];
    p.resize((HANDLER / 4) as usize, addi(0, 0, 0));
    p.extend([
        addi(9, 9, 1),
        csrrs(10, CSR_MEPC, 0),
        addi(10, 10, 4),
        csrrw(0, CSR_MEPC, 10),
        mret(),
    ]);
    let m = lockstep(&p, &[], "branch then trap", None, None, None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[9], 2, "both traps taken");
    assert_eq!(m.x[1], 0, "the skipped word never ran");
    assert_eq!(m.x[6], 0, "the loop ran out");
}

/// Branches guessed wrong, stepped one instruction at a time with
/// `dcsr.step` (issue 1300): every step that follows a wrong guess
/// enters debug mode at the right path's next instruction, with `dpc`
/// there, which lockstep checks at every entry and resume.
#[test]
fn branches_guessed_wrong_under_single_steps() {
    use vreteno32::isa::*;
    let p = vec![
        addi(6, 0, 4),
        beq(0, 0, 8),    // forward, taken: guessed wrong
        addi(1, 1, 100), // skipped
        addi(2, 2, 1),
        addi(6, 6, -1),
        bne(6, 0, -16), // backward: taken, then not, wrongly
        halt(),
    ];
    let plan = DebugPlan {
        halt_at: 2,
        hold: 3,
        resumes: 40,
        entries: RefCell::new(Vec::new()),
        writes: vec![(0, CSR_DCSR, 4)],
    };
    let m = lockstep(&p, &[], "branch steps", None, Some(&plan), None);
    assert_eq!(m.halted, Some(Halt::Break));
    assert_eq!(m.x[2], 4, "four rounds");
    assert_eq!(m.x[1], 0, "the skipped word never ran");
    let entries = plan.entries.borrow();
    assert!(entries.len() > 10, "stepped through: {}", entries.len());
    for (i, &e) in entries.iter().enumerate().skip(1) {
        assert_eq!(cause(e), 4, "entry {i} is a step: {e:#x}");
    }
}

/// Where the supervisor's line rises in
/// `an_interrupt_is_not_taken_after_mie_is_cleared`.
static RISE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// The line high for three cycles from [`RISE`].
fn three_from_rise(c: u64) -> bool {
    let r = RISE.load(std::sync::atomic::Ordering::Relaxed);
    (r..r + 3).contains(&c)
}

/// The core decides whether to take an interrupt a cycle before it
/// takes it (issue 1331). An instruction that clears `MIE` as the line
/// rises must not be followed by the interrupt it was decided on: the
/// decision is dropped after a write of the enables. The program sets
/// and clears `MIE` over and over while the line, the supervisor's
/// external one kept in machine mode, rises for three cycles at each
/// point in turn; every interrupt the core takes must be one `MIE`
/// allows where it is taken, which the run checks against the model.
#[test]
fn an_interrupt_is_not_taken_after_mie_is_cleared() {
    use vreteno32::isa::*;
    use vreteno32::program::Asm;
    let mut a = Asm::default();
    let (mh, wait) = (a.label(), a.label());
    a.wide(addi(8, 0, 0));
    a.abs(mh, |h| addi(31, 0, h as i32));
    a.wide(csrrw(0, CSR_MTVEC, 31));
    a.wide(addi(5, 0, SEXT as i32));
    a.wide(csrrw(0, CSR_MIE, 5));
    for _ in 0..48 {
        a.wide(csrrsi(0, CSR_MSTATUS, 8));
        a.wide(csrrci(0, CSR_MSTATUS, 8));
        // Not a CSR access: one would stall a cycle on entry and
        // hide what this looks for.
        a.wide(addi(9, 9, 1));
        a.wide(addi(9, 9, 1));
    }
    a.wide(halt());
    // The handler counts, waits for the line to fall, and returns with
    // `MIE` as it was.
    a.align();
    a.place(mh);
    a.wide(addi(8, 8, 1));
    a.place(wait);
    a.wide(csrrs(6, CSR_MIP, 0));
    a.wide(andi(6, 6, SEXT as i32));
    a.to(wait, |o| bne(6, 0, o));
    a.wide(mret());
    let words = a.words();
    let mut taken = 0;
    for rise in 30..70 {
        RISE.store(rise, std::sync::atomic::Ordering::Relaxed);
        let m = lockstep_with(
            &words,
            &[],
            &format!("mie cleared, line at {rise}"),
            None,
            None,
            None,
            Some(three_from_rise),
        );
        assert_eq!(m.halted, Some(Halt::Break));
        taken += m.x[8];
    }
    assert!(taken > 0, "the line was never taken at all");
}
