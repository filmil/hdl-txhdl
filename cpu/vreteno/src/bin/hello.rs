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
use txhdl::comp::{chan, join2, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use vreteno32::core::{Vreteno, Writeback};
use vreteno32::dmem::Dmem;
use vreteno32::router::Router;
use vreteno32::term::Terminal;
use vreteno32::timer::Timer;
use vreteno32::uart::Uart;

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
    let mut dmem = Dmem::with(hello_program::DATA);
    let mut router = Router::default();
    let mut timer = Timer::default();
    let mut uart = Uart::<4>::default();
    let (rst_out, rst) = signal::<Bit, DefaultClock>();
    let (irq_out, irq) = signal::<Bit, DefaultClock>();
    let (tirq_out, tirq) = signal::<Bit, DefaultClock>();
    let (tx_out, tx) = signal::<Bit, DefaultClock>();
    let (rx_out, rx) = signal::<Bit, DefaultClock>();
    let (uirq_out, uirq) = signal::<Bit, DefaultClock>();
    let (halt_out, halt) = signal::<Bit, DefaultClock>();
    let (instr_out, _instr) = signal::<U<32>, DefaultClock>();
    let (wb_out, _wb) = signal::<Writeback, DefaultClock>();
    let (req_tx, req_rx) = chan::<U<69>, DefaultClock>();
    let (resp_tx, resp_rx) = chan::<U<32>, DefaultClock>();
    let (dreq_tx, dreq_rx) = chan::<U<69>, DefaultClock>();
    let (dresp_tx, dresp_rx) = chan::<U<32>, DefaultClock>();
    let (treq_tx, treq_rx) = chan::<U<69>, DefaultClock>();
    let (tresp_tx, tresp_rx) = chan::<U<32>, DefaultClock>();
    let (ureq_tx, ureq_rx) = chan::<U<69>, DefaultClock>();
    let (uresp_tx, uresp_rx) = chan::<U<32>, DefaultClock>();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("rst", &rst);
        w.add("tx", &tx);
        w.add("ureq", &ureq_rx);
        w.add("uresp", &uresp_rx);
        w.add("cpu", &cpu);
        w.add("uart", &uart);
        w.add("halt", &halt);
        w.start();
    }
    let (rst_t, rst_u) = (rst.clone(), rst.clone());
    let mut sim = Running::new(join2(
        join2(
            join2(
                timer.run((rst_t, treq_rx), (tresp_tx, tirq_out)),
                uart.run((rst_u, rx, ureq_rx), (uresp_tx, tx_out, uirq_out)),
            ),
            dmem.run(dreq_rx, dresp_tx),
        ),
        join2(
            router.run(
                (req_rx, dresp_rx, tresp_rx, uresp_rx),
                (dreq_tx, treq_tx, ureq_tx, resp_tx),
            ),
            cpu.run(
                (rst, irq, tirq, resp_rx),
                (halt_out, instr_out, wb_out, req_tx),
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
