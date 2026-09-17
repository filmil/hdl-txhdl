// SPDX-License-Identifier: Apache-2.0
//! Run the demonstration program on the core, print a line per cycle,
//! write the waveform where `TXHDL_FST` points, and the VHDL of the
//! core, with the program in its instruction memory, where
//! `TXHDL_VHDL` points; then print the Verilog.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi_units, AxiHost, AxiPer};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge1};
use txhdl_parts::bus::router::Router3;
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
type Rtr =
    Router3<32, 32, 4, IW, 0x1000, 0xf000, 0x2000, 0xf000, 0x3000, 0xf000>;

/// The bridge the serial port sits behind: one AXI-Lite peripheral,
/// at the range the router gives the port.
type Serial = LiteBridge1<32, 32, 4, IW, 0x3000, 0xf000>;

fn main() {
    let program = demo();
    let mut cpu = Vreteno::with(&program);
    let (wb_pc, regs) = (cpu.wb_pc.clone(), cpu.regs.clone());
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
    let mut timer = Timer::<IW>::default();
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
        w.add("aw0", &dl.host_out.0);
        w.add("ar0", &dl.host_out.1);
        w.add("w0", &dl.host_out.2);
        w.add("b0", &dl.host_in.2);
        w.add("r0", &dl.host_in.3);
        w.add("aw1", &tl.host_out.0);
        w.add("ar1", &tl.host_out.1);
        w.add("w1", &tl.host_out.2);
        w.add("b1", &tl.host_in.2);
        w.add("r1", &tl.host_in.3);
        w.add("aw2", &ul.host_out.0);
        w.add("ar2", &ul.host_out.1);
        w.add("w2", &ul.host_out.2);
        w.add("b2", &ul.host_in.2);
        w.add("r2", &ul.host_in.3);
        // The transaction level at each peripheral.
        w.add("dreq", &dreq);
        w.add("dwd", &dwd);
        w.add("dans", &dans);
        w.add("drb", &drb);
        w.add("treq", &treq);
        w.add("twd", &twd);
        w.add("tans", &tans);
        w.add("trb", &trb);
        w.add("uaw", &uaw);
        w.add("uar", &uar);
        w.add("uw", &uw);
        w.add("ub", &ub);
        w.add("ur", &ur);
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
        w.start();
    }
    // The timer first: its line is a wire the core reads in the same
    // step, so the process that drives it runs before the one that
    // reads it. The channels between them do not care.
    let (rst_t, rst_u) = (rst.clone(), rst.clone());
    let mut sim = Running::new(join2(
        join2(
            join2(
                timer.run((rst_t, treq, twd), (tans, trb, tirq_out)),
                uart.run((rst_u, rx, uaw, uar, uw), (ub, ur, tx_out, uirq_out)),
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
    println!("{:>4} {:>6}   {:<22} {}", "t", "pc", "instruction", "writes");
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
    dmem.trace_as("req", "dreq");
    dmem.trace_as("wd", "dwd");
    dmem.trace_as("ans", "dans");
    dmem.trace_as("rb", "drb");
    let mut timer = Timer::<IW>::lowered("timer");
    timer.trace_as("req", "treq");
    timer.trace_as("wd", "twd");
    timer.trace_as("ans", "tans");
    timer.trace_as("rb", "trb");
    let mut uart4 = Uart::<4>::lowered("uart4");
    uart4.trace_as("aw", "uaw");
    uart4.trace_as("ar", "uar");
    uart4.trace_as("w", "uw");
    uart4.trace_as("b", "ub");
    uart4.trace_as("r", "ur");
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
        ("aw0", "uaw"),
        ("ar0", "uar"),
        ("w0", "uw"),
        ("b0", "ub"),
        ("r0", "ur"),
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
