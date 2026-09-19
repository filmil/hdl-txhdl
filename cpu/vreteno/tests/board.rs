// SPDX-License-Identifier: Apache-2.0
//! The board's design, as one lowered unit, run on the programs
//! compiled for the core: the greeting, which uses the data memory and
//! the serial port; the DDR3 test, which uses the memory's region
//! through the bridge and the controller's model; and input by
//! interrupt, which takes the serial port's receive interrupt through
//! the interrupt controller. And its netlist, which holds the memory
//! controller as a foreign module.
use txhdl::comp::{pad, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use vreteno32::board::Board;
use vreteno32::core::Vreteno;
use vreteno32::dmem::Dmem;
use vreteno32::term::Terminal;

/// The serial port's divider in these runs: four cycles a bit, as the
/// demonstration has it.
type TestBoard = Board<4, 1, 0>;

/// What a run came to: what the serial line said, and the cycle the
/// core halted on.
struct Ran {
    said: String,
    halted_at: Option<u64>,
}

/// Run `text` with `data` in the data memory, on the board's design,
/// for at most `limit` cycles, with a terminal on the serial line that
/// types `reply` once the core has said a line.
fn run(text: &[u32], data: &[u8], reply: &[u8], limit: u64) -> Ran {
    let mut board = TestBoard {
        cpu: Vreteno::with(text),
        dmem: Dmem::with(data),
        ..Default::default()
    };
    let (rst_o, rst) = signal::<Bit, DefaultClock>();
    let (irq_o, irq) = signal::<Bit, DefaultClock>();
    let (rx_o, rx) = signal::<Bit, DefaultClock>();
    let quiet = || signal::<Bit, DefaultClock>().1;
    let bit = || signal::<Bit, DefaultClock>().0;
    let (halt_o, halt) = signal::<Bit, DefaultClock>();
    let (tx_o, tx) = signal::<Bit, DefaultClock>();
    let mut sim = Running::new(board.run(
        (rst, irq, rx, quiet(), quiet(), quiet(), quiet()),
        (
            halt_o,
            tx_o,
            // The pulse width modulator's four channels, which this
            // run does not look at.
            signal::<U<4>, DefaultClock>().0,
            bit(),
            bit(),
            bit(),
            bit(),
            bit(),
            bit(),
            bit(),
            bit(),
            bit(),
            signal::<U<15>, DefaultClock>().0,
            signal::<U<3>, DefaultClock>().0,
            signal::<U<4>, DefaultClock>().0,
            bit(),
            pad::<U<32>, DefaultClock>(),
            pad::<U<4>, DefaultClock>(),
            pad::<U<4>, DefaultClock>(),
        ),
    ));
    rst_o.set(Bit::One);
    sim.cycle();
    rst_o.set(Bit::Zero);
    irq_o.set(Bit::Zero);
    rx_o.set(Bit::One);
    let mut term = Terminal::new(reply);
    let mut halted_at = None;
    for cycle in 0..limit {
        sim.cycle();
        term.see(tx.get().to_bool());
        rx_o.set(Bit::from_bool(term.level()));
        if halt.get().to_bool() {
            halted_at = Some(cycle);
            break;
        }
    }
    // The last byte is still going out when the core halts.
    for _ in 0..64 {
        sim.cycle();
        term.see(tx.get().to_bool());
    }
    Ran {
        said: term.said.clone(),
        halted_at,
    }
}

#[test]
fn the_greeting_runs_on_the_board() {
    let ran = run(hello_program::TEXT, hello_program::DATA, b"", 8000);
    assert_eq!(ran.said, "hello from rust\n");
    assert!(ran.halted_at.is_some(), "the core halted itself");
}

#[test]
fn the_memory_test_runs_on_the_board() {
    let ran = run(ddr3_program::TEXT, ddr3_program::DATA, b"", 40000);
    assert_eq!(ran.said, "ddr3 ok\n");
    assert!(ran.halted_at.is_some(), "the core halted itself");
}

/// The terminal types four bytes, and the program takes each through
/// the interrupt controller, one interrupt a byte, never polling the
/// serial port for input.
#[test]
fn input_comes_by_interrupt_one_byte_each() {
    let ran = run(irq_program::TEXT, irq_program::DATA, b"ping", 20000);
    assert_eq!(ran.said, "ready\ngot ping in 4 interrupts\n");
    assert!(ran.halted_at.is_some(), "the core halted itself");
}

/// One module holds the rest, and the controller is an instance of its
/// wrapper that the netlist does not write, with the memory's pads
/// running out to the board's own ports.
#[test]
fn the_netlist_holds_the_controller() {
    let v = TestBoard::verilog("board");
    assert!(v.contains("module board("), "the top");
    assert!(v.contains("module board_cpu("), "the core");
    assert!(v.contains("module board_ddr3_bridge("), "the bridge");
    assert!(v.contains("ddr3_wb32 #("), "the controller");
    assert!(!v.contains("module ddr3_wb32"), "not written");
    assert!(v.contains("inout [31:0] dq"), "the data pads");
}
