// SPDX-License-Identifier: Apache-2.0
//! The whole machine, for a program compiled to run on it.
//!
//! The demonstration in `main.rs` wires the core, the link, the
//! router and the three peripherals for one program and prints a line
//! per cycle. A compiled program wants the same machine and a
//! different question: what came out of the serial port, and did the
//! core stop by itself. That is this, in one call, so that a program
//! in any language reaching the core is run and judged the same way.
use crate::core::{Vreteno, Writeback};
use crate::dmem::Dmem;
use crate::rom::Rom;
use crate::term::Terminal;
use crate::timer::Timer;
use crate::uart::Uart;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi_units, AxiHost, AxiPer};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge1};
use txhdl_parts::bus::router::Router4;

/// The link, as the demonstration has it: thirty-two bit addresses
/// and words, four lanes, two-bit identifiers.
const IW: usize = 2;
const NIDS: usize = 4;

/// The address map, stated in the router's type.
type Rtr = Router4<
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
    0x0000,
    0xf000,
>;

/// The bridge the serial port sits behind: one AXI-Lite peripheral,
/// at the range the router gives the port.
type Serial = LiteBridge1<32, 32, 4, IW, 0x3000, 0xf000>;

/// What a run came to: what a terminal on the serial line heard, and
/// the cycle the core halted itself on, if it did.
pub struct Ran {
    pub said: String,
    pub halted_at: Option<u64>,
    /// The first words of the data memory at the end of the run, so
    /// that a program can answer a test by writing rather than by
    /// printing, which is what a test of a number wants.
    pub mem: Vec<u32>,
}

/// Run `text` on the core with `data` in its memory, for at most
/// `limit` cycles.
pub fn run(text: &[u32], data: &[u8], limit: u64) -> Ran {
    let mut cpu = Vreteno::with(text);
    // The data memory's initial bytes, and the boot memory on the bus:
    // the same words the core fetches, readable by a load and not
    // writable, so a program's constants can sit beside its code.
    let mut dmem = Dmem::<IW>::with(data);
    let mut rom = Rom::<IW>::with(text);
    let lanes = (
        dmem.lane0.clone(),
        dmem.lane1.clone(),
        dmem.lane2.clone(),
        dmem.lane3.clone(),
    );
    let (rst_out, rst) = signal::<Bit, DefaultClock>();
    let (irq_out, irq) = signal::<Bit, DefaultClock>();
    let (tirq_out, tirq) = signal::<Bit, DefaultClock>();
    let (sirq_out, sirq) = signal::<Bit, DefaultClock>();
    let (tx_out, tx) = signal::<Bit, DefaultClock>();
    let (rx_out, rx) = signal::<Bit, DefaultClock>();
    // The port's interrupt, which these programs do not use: none of
    // them waits for input or installs a handler.
    let (uirq_out, _uirq) = signal::<Bit, DefaultClock>();
    let (halt_out, halt) = signal::<Bit, DefaultClock>();
    let (instr_out, _instr) = signal::<U<32>, DefaultClock>();
    let (wb_out, _wb) = signal::<Writeback, DefaultClock>();
    // The core's link and one per peripheral, with the router between
    // the core's tracker and the three peripherals'.
    let cl = axi_units::<32, 32, 4, IW>();
    let dl = axi_units::<32, 32, 4, IW>();
    let tl = axi_units::<32, 32, 4, IW>();
    let ul = axi_units::<32, 32, 4, IW>();
    let rl = axi_units::<32, 32, 4, IW>();
    let (issue, wbeat, release, grant, cdone, crdata) = cl.host_client;
    let (dreq, dwd, dans, drb) = dl.per_client;
    let (treq, twd, tans, trb) = tl.per_client;
    let (rreq, rwd, rans, rrb) = rl.per_client;
    // The serial port is an AXI-Lite peripheral, behind a bridge
    // that takes the AXI4 channels the router gives it.
    let sl = axi_lite::<32, 32, 4>();
    let (uaw, uar, uw, ub, ur) = sl.per;
    let (baw, bar, bw, bb, br) = sl.host;
    let mut axi_host = AxiHost::<32, 32, 4, IW, NIDS>::default();
    let mut dper = AxiPer::<32, 32, 4, IW>::default();
    let mut tper = AxiPer::<32, 32, 4, IW>::default();
    let mut rper = AxiPer::<32, 32, 4, IW>::default();
    let mut ubridge = Serial::default();
    let mut router = Rtr::default();
    let mut timer = Timer::<IW>::default();
    let mut uart = Uart::<4>::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("rst", &rst);
        w.add("tx", &tx);
        w.add("issue", &issue);
        w.add("grant", &grant);
        w.add("cpu", &cpu);
        w.add("uart", &uart);
        w.add("halt", &halt);
        w.start();
    }
    let (rst_t, rst_u) = (rst.clone(), rst.clone());
    let mut sim = Running::new(join2(
        join2(
            join2(
                timer.run((rst_t, treq, twd), (tans, trb, tirq_out, sirq_out)),
                uart.run((rst_u, rx, uaw, uar, uw), (ub, ur, tx_out, uirq_out)),
            ),
            join2(
                join2(
                    dmem.run((dreq, dwd), (dans, drb)),
                    rom.run((rreq, rwd), (rans, rrb)),
                ),
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
                        rl.host_in.2,
                        rl.host_in.3,
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
                        rl.host_out.0,
                        rl.host_out.1,
                        rl.host_out.2,
                        cl.per_out.2,
                        cl.per_out.3,
                    ),
                ),
            ),
            join2(
                join2(
                    join2(
                        dper.run(dl.per_in, dl.per_out),
                        tper.run(tl.per_in, tl.per_out),
                    ),
                    rper.run(rl.per_in, rl.per_out),
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
    irq_out.set(Bit::Zero);
    rx_out.set(Bit::One);
    // A terminal that never types, only listens.
    let mut term = Terminal::new(b"");
    let mut halted_at = None;
    for cycle in 0..limit {
        sim.cycle();
        term.see(tx.get().to_bool());
        rx_out.set(Bit::from_bool(term.level()));
        if halt.get().to_bool() {
            halted_at = Some(cycle);
            break;
        }
    }
    // The core stops the moment it writes `mhalt`, and the last byte
    // is still going out a bit at a time: a frame is ten bits of four
    // cycles. The line is read to the end of it before the run is
    // judged, or the last character would always be missing.
    for _ in 0..64 {
        sim.cycle();
        term.see(tx.get().to_bool());
    }
    stop();
    let word = |a: usize| -> u32 {
        (lanes.0.read(a).raw() as u32)
            | (lanes.1.read(a).raw() as u32) << 8
            | (lanes.2.read(a).raw() as u32) << 16
            | (lanes.3.read(a).raw() as u32) << 24
    };
    Ran {
        said: term.said.clone(),
        halted_at,
        mem: (0..16).map(word).collect(),
    }
}

/// Say what a run came to, and hold it to what the program was
/// written to say. A program that says nothing fails rather than
/// passes quietly.
pub fn expect(what: &str, ran: &Ran, expected: &str) {
    println!("{what} said: {:?}", ran.said);
    match ran.halted_at {
        Some(c) => println!("it halted on its own at cycle {c}"),
        None => println!("it never halted"),
    }
    assert_eq!(ran.said, expected, "what {what} printed");
    assert!(ran.halted_at.is_some(), "{what} halted the core");
}
