// SPDX-License-Identifier: Apache-2.0
//! Run the Rust program on the core, and check that it says what it
//! was written to say.
//!
//! The program in `cpu/vreteno/rust/hello.rs` is compiled for Vreteno
//! itself, by the toolchain `MODULE.bazel` fetches, and turned into
//! an image by `//tools/elf2vreteno`. This is the other half: the
//! whole machine, the core with that image in its instruction memory
//! and the image's constants in its data memory, running until the
//! program's `ebreak` halts it, with a terminal on the serial line
//! reading what comes out.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi_units, AxiHost, AxiPer};
use txhdl_parts::bus::router::Router3;
use vreteno32::core::{Vreteno, Writeback};
use vreteno32::dmem::Dmem;
use vreteno32::term::Terminal;
use vreteno32::timer::Timer;
use vreteno32::uart::Uart;

/// The link, as the demonstration has it: thirty-two bit addresses
/// and words, four lanes, two-bit identifiers.
const IW: usize = 2;
const NIDS: usize = 4;

/// The address map, stated in the router's type.
type Rtr =
    Router3<32, 32, 4, IW, 0x1000, 0xf000, 0x2000, 0xf000, 0x3000, 0xf000>;

/// What the program is written to print. The check is against this
/// and not against whatever came out, so a program that says nothing
/// fails rather than passes quietly.
const EXPECTED: &str = "hello from rust\n";

fn main() {
    let (said, halted_at) = run();
    println!("the core said: {said:?}");
    match halted_at {
        Some(c) => println!("it halted on its own at cycle {c}"),
        None => println!("it never halted"),
    }
    check(&said, halted_at);
    println!("Rust, compiled for Vreteno, ran on it and said its line.");
}

/// What the run has to have done, in one place, so that the binary and
/// the test judge it by the same rule.
fn check(said: &str, halted_at: Option<u64>) {
    assert_eq!(said, EXPECTED, "what the Rust program printed");
    assert!(
        halted_at.is_some(),
        "the program's `ebreak` halted the core"
    );
}

/// The whole machine for one run: what the terminal heard, and the
/// cycle the core halted itself on, if it did.
fn run() -> (String, Option<u64>) {
    let mut cpu = Vreteno::with(hello_program::TEXT);
    // The constants the program reads, in the data memory before the
    // first cycle. Nothing on this machine could put them there later:
    // the instruction memory the program lives in is not on the bus,
    // so a load never reaches it.
    let mut dmem = Dmem::<IW>::with(hello_program::DATA);
    let (rst_out, rst) = signal::<Bit, DefaultClock>();
    let (irq_out, irq) = signal::<Bit, DefaultClock>();
    let (tirq_out, tirq) = signal::<Bit, DefaultClock>();
    let (tx_out, tx) = signal::<Bit, DefaultClock>();
    let (rx_out, rx) = signal::<Bit, DefaultClock>();
    // The port's interrupt, which this program does not use: it
    // neither waits for input nor installs a handler.
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
    let (issue, wbeat, release, grant, cdone, crdata) = cl.host_client;
    let (dreq, dwd, dans, drb) = dl.per_client;
    let (treq, twd, tans, trb) = tl.per_client;
    let (ureq, uwd, uans, urb) = ul.per_client;
    let mut axi_host = AxiHost::<32, 32, 4, IW, NIDS>::default();
    let mut dper = AxiPer::<32, 32, 4, IW>::default();
    let mut tper = AxiPer::<32, 32, 4, IW>::default();
    let mut uper = AxiPer::<32, 32, 4, IW>::default();
    let mut router = Rtr::default();
    let mut timer = Timer::<IW>::default();
    let mut uart = Uart::<4, IW>::default();
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
    irq_out.set(Bit::Zero);
    rx_out.set(Bit::One);
    // A terminal that never types, only listens.
    let mut term = Terminal::new(b"");
    // A byte is ten bits of four cycles, so the line takes about six
    // hundred and fifty; the halt ends the run before the limit unless
    // something is wrong.
    let mut halted_at = None;
    for cycle in 0..4000u64 {
        sim.cycle();
        term.see(tx.get().to_bool());
        rx_out.set(Bit::from_bool(term.level()));
        if halt.get().to_bool() {
            halted_at = Some(cycle);
            break;
        }
    }
    // The core stops the moment it reaches `ebreak`, and the last byte
    // is still going out a bit at a time: a frame is ten bits of four
    // cycles. The line is read to the end of it before the run is
    // judged, or the last character would always be missing.
    for _ in 0..64 {
        sim.cycle();
        term.see(tx.get().to_bool());
    }
    stop();
    (term.said.clone(), halted_at)
}

/// The same run, as a test, so that `bazel test //...` checks that
/// Rust compiled for the core still runs on it.
#[cfg(test)]
mod tests {
    use super::{check, run};

    #[test]
    fn rust_runs_on_vreteno() {
        let (said, halted_at) = run();
        check(&said, halted_at);
    }
}
