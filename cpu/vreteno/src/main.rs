// SPDX-License-Identifier: Apache-2.0
//! Run the demonstration program on the core, print a line per cycle,
//! write the waveform where `TXHDL_FST` points, and the VHDL of the
//! core, with the program in its instruction memory, where
//! `TXHDL_VHDL` points; then print the Verilog.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, DefaultClock, Running, Unit};
use txhdl::map::AddrMap;
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi_units, AxiHost, AxiPer, PerPort};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge, LitePort};
use txhdl_parts::bus::router::Router;
use vreteno32::core::{Vreteno, Writeback};
use vreteno32::dmem::Dmem;
use vreteno32::isa::disasm;
use vreteno32::program::demo;
use vreteno32::term::Terminal;
use vreteno32::timer::Timer;
use vreteno32::uart::Uart;

/// The link: thirty-two bit addresses and words, four lanes, and
/// two-bit identifiers, four of them. The core has one load in flight
/// and posts its stores, and it hands an identifier back as its
/// answer arrives, so four is more than it uses.
const IW: usize = 2;
const NIDS: usize = 4;

/// The address map, as the router's type states it: the data memory,
/// the timer and the serial port, a nibble each, and every other
/// address a hole the router answers itself.
/// The address map: the data memory at its page, the timer at
/// `0x0200_0000`, and the serial port at `0x3000`.
struct RunMap;

impl AddrMap<3> for RunMap {
    const RANGES: [(usize, usize); 3] = [
        (0x1000, 0xf000),
        (0x0200_0000, 0xffff_0000),
        (0x3000, 0xf000),
    ];
}

type Rtr = Router<3, RunMap, 32, 32, 4, IW>;

/// The bridge the serial port sits behind: one AXI-Lite peripheral,
/// at the range the router gives the port.
type Serial = LiteBridge<1, SerialMap, 32, 32, 4, IW>;

/// Where the serial port is: a nibble of the address space at 0x3000.
pub struct SerialMap;

impl AddrMap<1> for SerialMap {
    const RANGES: [(usize, usize); 1] = [(0x3000, 0xf000)];
}

fn main() {
    let program = demo();
    let mut cpu = Vreteno::with(&program);
    let (wb_pc, regs) = (cpu.wb_pc, cpu.regs.clone());
    let mut dmem = Dmem::default();
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
    let (rst_out, rst) = signal::<Bit, DefaultClock>();
    let (irq_out, irq) = signal::<Bit, DefaultClock>();
    // The bus: a request channel out of the core into the router, a
    // response channel back, and a channel each way from the router to
    // each device, the timer with its interrupt line and the serial
    // port with its two lines and its interrupt, which the core's line
    // carries. The port's bits are four cycles each here, so a byte
    // takes forty.
    let (tirq_out, tirq) = signal::<Bit, DefaultClock>();
    let (sirq_out, sirq) = signal::<Bit, DefaultClock>();
    let (tx_out, tx) = signal::<Bit, DefaultClock>();
    let (rx_out, rx) = signal::<Bit, DefaultClock>();
    let (uirq_out, uirq) = signal::<Bit, DefaultClock>();
    // The core's link, and one per peripheral. The core is a host
    // client written as hardware, so it holds its link's channel ends
    // itself rather than a `Host`; each peripheral likewise. The
    // router stands between the core's tracker and the three
    // peripherals', and nothing on either side knows it is there.
    let cl = axi_units::<32, 32, 4, IW>();
    let dl = axi_units::<32, 32, 4, IW>();
    let tl = axi_units::<32, 32, 4, IW>();
    let ul = axi_units::<32, 32, 4, IW>();
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
    let mut timer = Timer::<IW>::default();
    let mut uart = Uart::<4>::default();
    let (halt_out, halt) = signal::<Bit, DefaultClock>();
    // No debugger here: its request lines stay low and what the
    // core says about debug mode is not read (issue 154).
    let (_haltreq_o, haltreq) = signal::<Bit, DefaultClock>();
    let (_resumereq_o, resumereq) = signal::<Bit, DefaultClock>();
    let (debug_o, dbg) = signal::<Bit, DefaultClock>();
    // The debug module's register access, absent here but traced, since
    // the testbench reads every port of the core from the wave.
    let (_dbg_regno_o, dbg_regno) = signal::<U<16>, DefaultClock>();
    let (_dbg_wdata_o, dbg_wdata) = signal::<U<32>, DefaultClock>();
    let (_dbg_we_o, dbg_we) = signal::<Bit, DefaultClock>();
    let (dbg_rdata_o, dbg_rdata) = signal::<U<32>, DefaultClock>();
    let (instr_out, instr) = signal::<U<32>, DefaultClock>();
    let (wb_out, wb) = signal::<Writeback, DefaultClock>();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("rst", &rst);
        w.add("irq", &irq);
        w.add("tirq", &tirq);
        // The software interrupt, which the timer raises from `msip`
        // and the core takes as its third. It is a port of both
        // lowered units, so a trace without it leaves the testbenches
        // for them with a port they cannot drive; `fst2tb` says so and
        // refuses. See issue 270.
        w.add("sirq", &sirq);
        w.add("tx", &tx);
        w.add("rx", &rx);
        w.add("uirq", &uirq);
        // The transaction level between the core and its tracker.
        w.add("issue", &issue);
        w.add("wbeat", &wbeat);
        w.add("release", &release);
        w.add("grant", &grant);
        w.add("done", &cdone);
        w.add("rdata", &crdata);
        // The five AXI channels the router sits in, at the core's
        // side and at each peripheral's.
        w.add("aw", &cl.host_out.0);
        w.add("ar", &cl.host_out.1);
        w.add("w", &cl.host_out.2);
        w.add("b", &cl.host_in.2);
        w.add("r", &cl.host_in.3);
        w.add("aws_0", &dl.host_out.0);
        w.add("ars_0", &dl.host_out.1);
        w.add("ws_0", &dl.host_out.2);
        w.add("bs_0", &dl.host_in.2);
        w.add("rs_0", &dl.host_in.3);
        w.add("aws_1", &tl.host_out.0);
        w.add("ars_1", &tl.host_out.1);
        w.add("ws_1", &tl.host_out.2);
        w.add("bs_1", &tl.host_in.2);
        w.add("rs_1", &tl.host_in.3);
        w.add("aws_2", &ul.host_out.0);
        w.add("ars_2", &ul.host_out.1);
        w.add("ws_2", &ul.host_out.2);
        w.add("bs_2", &ul.host_in.2);
        w.add("rs_2", &ul.host_in.3);
        // The same five channels under the bridge's own port names, so
        // that its netlist is checked against the trace as well as the
        // router's.
        w.add("aw2", &ul.host_out.0);
        w.add("ar2", &ul.host_out.1);
        w.add("w2", &ul.host_out.2);
        w.add("b2", &ul.host_in.2);
        w.add("r2", &ul.host_in.3);
        // The transaction level at each peripheral.
        w.add("dreq", &dbus.req);
        w.add("dwd", &dbus.w);
        w.add("dans", &dbus.ans);
        w.add("drb", &dbus.r);
        w.add("treq", &tbus.req);
        w.add("twd", &tbus.w);
        w.add("tans", &tbus.ans);
        w.add("trb", &tbus.r);
        w.add("uaw", &ubus.aw);
        w.add("uar", &ubus.ar);
        w.add("uw", &ubus.w);
        w.add("ub", &ubus.b);
        w.add("ur", &ubus.r);
        w.add("cpu", &cpu);
        w.add("dmem", &dmem);
        w.add("router", &router);
        w.add("timer", &timer);
        w.add("uart", &uart);
        w.add("axi_host", &axi_host);
        w.add("dper", &dper);
        w.add("tper", &tper);
        w.add("ubridge", &ubridge);
        w.add("instr", &instr);
        w.add("wb", &wb);
        w.add("halt", &halt);
        // The debugger's lines, quiet here, traced because the
        // testbench reads every port of the core from the wave.
        w.add("haltreq", &haltreq);
        w.add("resumereq", &resumereq);
        w.add("dbg", &dbg);
        w.add("dbg_regno", &dbg_regno);
        w.add("dbg_wdata", &dbg_wdata);
        w.add("dbg_we", &dbg_we);
        w.add("dbg_rdata", &dbg_rdata);
        w.start();
    }
    // The timer first: its line is a wire the core reads in the same
    // step, so the process that drives it runs before the one that
    // reads it. The channels between them do not care.
    let (rst_t, rst_u) = (rst.clone(), rst.clone());
    let mut sim = Running::new(join2(
        join2(
            join2(
                timer.run(tbus, (rst_t, tirq_out, sirq_out)),
                uart.run(ubus, (rst_u, rx, tx_out, uirq_out)),
            ),
            join2(
                dmem.run(dbus, ()),
                cpu.run(
                    (
                        rst, irq, tirq, sirq, crdata, cdone, grant, haltreq,
                        resumereq, dbg_regno, dbg_wdata, dbg_we,
                    ),
                    (
                        halt_out,
                        instr_out,
                        wb_out,
                        issue,
                        wbeat,
                        release,
                        debug_o,
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
                        [dl.host_in.2, tl.host_in.2, ul.host_in.2],
                        [dl.host_in.3, tl.host_in.3, ul.host_in.3],
                    ),
                    (
                        [dl.host_out.0, tl.host_out.0, ul.host_out.0],
                        [dl.host_out.1, tl.host_out.1, ul.host_out.1],
                        [dl.host_out.2, tl.host_out.2, ul.host_out.2],
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
                    (ul.per_in.0, ul.per_in.1, ul.per_in.2, [bb], [br]),
                    ([baw], [bar], [bw], ul.per_out.2, ul.per_out.3),
                ),
            ),
        ),
    ));
    rst_out.set(Bit::One);
    sim.cycle();
    rst_out.set(Bit::Zero);
    // The header takes the widths of the rows below it, so it is
    // written with them rather than spaced out by hand.
    #[allow(clippy::print_literal)]
    {
        println!(
            "{:>4} {:>6}   {:<22} {}",
            "t", "pc", "instruction", "writes"
        );
    }
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
        irq_out.set((cycle == irq_at) | uirq.get());
        rx_out.set(term.level());
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
        // The core expands a compressed instruction in the fetch, so
        // what retires is the one it stands for; the program says
        // which were compressed, and those are marked with a c.
        let short = vreteno32::model::fetch(&program, at)
            .is_some_and(|(_, len)| len == 2);
        let mark = if short { "c" } else { " " };
        let text = disasm(instr.get().raw() as u32);
        println!("{:>4} {at:#06x} {mark} {text:<22} {wrote}", now());
    }
    // The last byte is still going out when the core halts; let the
    // port finish its frame.
    let mut grace = 0;
    while term.busy() && grace < 64 {
        rx_out.set(term.level());
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
    let mut lowered = Vreteno::<IW>::lowered("vreteno");
    let words: Vec<u128> = program.iter().map(|&w| w as u128).collect();
    lowered.init("imem", &words);
    // The router and the three peripherals, each told under which
    // scope the run traced its channels, since a channel two units
    // share has a port name of its own on each side; the serial port
    // again at the board's baud rate; and the two trackers, which the
    // AXI document already checks but which the board needs too.
    let router = Rtr::lowered("router");
    let mut dmem = Dmem::<IW>::lowered("dmem");
    dmem.trace_as("bus_req", "dreq");
    dmem.trace_as("bus_w", "dwd");
    dmem.trace_as("bus_ans", "dans");
    dmem.trace_as("bus_r", "drb");
    let mut timer = Timer::<IW>::lowered("timer");
    timer.trace_as("bus_req", "treq");
    timer.trace_as("bus_w", "twd");
    timer.trace_as("bus_ans", "tans");
    timer.trace_as("bus_r", "trb");
    let mut uart4 = Uart::<4>::lowered("uart4");
    uart4.trace_as("bus_aw", "uaw");
    uart4.trace_as("bus_ar", "uar");
    uart4.trace_as("bus_w", "uw");
    uart4.trace_as("bus_b", "ub");
    uart4.trace_as("bus_r", "ur");
    uart4.trace_as("irq", "uirq");
    // The port's bridge: the AXI4 side under the router's names for
    // the third peripheral, the AXI-Lite side under the port's.
    let mut ubr = Serial::lowered("ubridge");
    for (port, scope) in [
        ("aw", "aw2"),
        ("ar", "ar2"),
        ("w", "w2"),
        ("b", "b2"),
        ("r", "r2"),
        ("aws_0", "uaw"),
        ("ars_0", "uar"),
        ("ws_0", "uw"),
        ("bs_0", "ub"),
        ("rs_0", "ur"),
    ] {
        ubr.trace_as(port, scope);
    }
    let uart = Uart::<868>::lowered("uart");
    let axi_host = AxiHost::<32, 32, 4, IW, NIDS>::lowered("axi_host");
    let mut dper = AxiPer::<32, 32, 4, IW>::lowered("axi_per");
    dper.trace_as("aw", "aw0");
    dper.trace_as("ar", "ar0");
    dper.trace_as("w", "w0");
    dper.trace_as("b", "b0");
    dper.trace_as("r", "r0");
    dper.trace_as("req", "dreq");
    dper.trace_as("wd", "dwd");
    dper.trace_as("ans", "dans");
    dper.trace_as("rb", "drb");
    txhdl::netlist::write_netlists_from_env(&[
        &lowered, &router, &dmem, &timer, &uart4, &ubr, &uart, &axi_host, &dper,
    ]);
    print!(
        "\n{}\n{}\n{}\n{}\n{}",
        lowered.verilog(),
        router.verilog(),
        dmem.verilog(),
        timer.verilog(),
        uart.verilog()
    );
}
