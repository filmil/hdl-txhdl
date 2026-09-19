// SPDX-License-Identifier: Apache-2.0
//! The board's design, as one lowered unit, run on the programs
//! compiled for the core: the greeting, which uses the data memory and
//! the serial port; the DDR3 test, which uses the memory's region
//! through the bridge and the controller's model; and input by
//! interrupt, which takes the serial port's receive interrupt through
//! the interrupt controller. And its netlist, which holds the memory
//! controller as a foreign module.
use txhdl::comp::{chan, pad, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi_lite::{LiteAr, LiteAw, LiteB, LiteR, LiteW};
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
    /// How many bytes of the stream the terminal managed to type, the
    /// cycle the last one went out on, and how long the run was.
    typed: usize,
    typed_at: u64,
    ran_for: u64,
    halted_at: Option<u64>,
}

/// Run `text` with `data` in the data memory, on the board's design,
/// for at most `limit` cycles, with a terminal on the serial line that
/// types `reply` once the core has said a line.
fn run(text: &[u32], data: &[u8], reply: &[u8], limit: u64) -> Ran {
    run_paced(text, data, reply, &[], limit)
}

/// The same, with the terminal typing in blocks of `block` bytes and
/// waiting for a byte back between them, which is how a sender talks
/// to the loader.
fn run_paced(
    text: &[u32],
    data: &[u8],
    reply: &[u8],
    blocks: &[usize],
    limit: u64,
) -> Ran {
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
    // The third slot of the page at `0x3000` is tied off: nothing in
    // these runs writes to `0x3200`, and a run that did would wait on
    // an answer that never comes, which is what the board top's own
    // tie-off does as well.
    let mut sim = Running::new(board.run(
        (
            rst,
            irq,
            rx,
            quiet(),
            quiet(),
            quiet(),
            quiet(),
            chan::<LiteB, DefaultClock>().1,
            chan::<LiteR<32>, DefaultClock>().1,
        ),
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
            chan::<LiteAw<32>, DefaultClock>().0,
            chan::<LiteAr<32>, DefaultClock>().0,
            chan::<LiteW<32, 4>, DefaultClock>().0,
        ),
    ));
    rst_o.set(Bit::One);
    sim.cycle();
    rst_o.set(Bit::Zero);
    irq_o.set(Bit::Zero);
    rx_o.set(Bit::One);
    let mut term = if blocks.is_empty() {
        Terminal::new(reply)
    } else {
        Terminal::paced(reply, blocks)
    };
    let mut halted_at = None;
    let mut typed_was = 0;
    let mut typed_at = 0;
    let mut ran_for = 0;
    for cycle in 0..limit {
        sim.cycle();
        term.see(tx.get().to_bool());
        rx_o.set(Bit::from_bool(term.level()));
        ran_for = cycle;
        if term.typed() != typed_was {
            typed_was = term.typed();
            typed_at = cycle;
        }
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
        typed: term.typed(),
        typed_at,
        ran_for,
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

/// A program for the loader to load: it says `hi` on the serial port
/// and halts. It is assembled here rather than compiled, because a
/// compiled image is linked for the boot memory and this one runs
/// wherever the stream says, which is what a loader is for.
fn payload() -> Vec<u32> {
    use vreteno32::isa::{addi, andi, bne, halt, lui, lw, sw, UART_BASE};
    let mut a = vreteno32::program::Asm::default();
    a.emit(lui(1, UART_BASE >> 12)); // x1 = the serial port
    for byte in b"hi\n" {
        // Wait while the port is busy, then write the byte.
        let wait = a.label();
        a.place(wait);
        a.emit(lw(2, 1, 4)); // x2 = the status
        a.emit(andi(2, 2, 1)); // busy?
        a.to(wait, |off| bne(2, 0, off));
        a.emit(addi(3, 0, *byte as i32));
        a.emit(sw(3, 1, 0)); // the byte goes out
    }
    a.emit(halt());
    a.words()
}

/// Where the sender pauses for the loader's acknowledgement: the
/// header, then every two words, then the checksum on its own. Two is
/// `BLOCK_WORDS` in the loader, and the two have to agree.
fn blocks(words: usize) -> Vec<usize> {
    let mut out = vec![12];
    let mut left = words;
    while left > 0 {
        let take = if left > 1 { 1 } else { left };
        out.push(take * 4);
        left -= take;
    }
    out.push(4);
    out
}

/// The stream the loader takes: the magic word, the address, the
/// length, the words and their sum, every number least significant
/// byte first.
fn stream(addr: u32, words: &[u32]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut word = |w: u32| out.extend_from_slice(&w.to_le_bytes());
    word(0x444c_5854);
    word(addr);
    word(words.len() as u32 * 4);
    let mut sum: u32 = 0;
    for w in words {
        word(*w);
        sum = sum.wrapping_add(*w);
    }
    word(sum);
    out
}

/// The loader takes a program off the serial port, writes it into the
/// memory's region and jumps to it, and the program runs.
#[test]
fn the_loader_loads_a_program_and_runs_it() {
    let addr = 0x4000_0000;
    let ran = run_paced(
        boot_program::TEXT,
        boot_program::DATA,
        &stream(addr, &payload()),
        &blocks(payload().len()),
        400_000,
    );
    // The loader says `boot`, then `load` when the header arrives, then
    // one `K` for the header and one for each word, then `ok` with the
    // address it jumps to; the program it loaded says `hi`.
    let acks = "K".repeat(payload().len() + 1);
    assert_eq!(
        ran.said,
        format!("boot\nload\n{acks}ok 40000000\nhi\n"),
        "typed {} of {} bytes, the last at cycle {} of {}",
        ran.typed,
        stream(addr, &payload()).len(),
        ran.typed_at,
        ran.ran_for
    );
    assert!(ran.halted_at.is_some(), "the loaded program halted");
}

/// A stream whose checksum does not match is refused, and the loader
/// waits for another rather than jumping into whatever arrived.
#[test]
fn the_loader_refuses_a_stream_whose_sum_is_wrong() {
    let addr = 0x4000_0000;
    let mut bytes = stream(addr, &payload());
    let last = bytes.len() - 4;
    bytes[last] ^= 0xff;
    let ran = run_paced(
        boot_program::TEXT,
        boot_program::DATA,
        &bytes,
        &blocks(payload().len()),
        400_000,
    );
    assert!(ran.said.contains("bad sum "), "{}", ran.said);
    assert!(ran.said.ends_with("boot\n"), "it waits for another: {}", ran.said);
    assert!(ran.halted_at.is_none(), "nothing was jumped into");
}

/// An address outside the memory's region is refused, so a stream
/// cannot write over the peripherals.
#[test]
fn the_loader_refuses_an_address_outside_the_memory() {
    let ran = run_paced(
        boot_program::TEXT,
        boot_program::DATA,
        &stream(0x3000, &payload()),
        &blocks(payload().len()),
        200_000,
    );
    assert!(ran.said.starts_with("boot\nload\nbad len "), "{}", ran.said);
    assert!(ran.halted_at.is_none(), "nothing was jumped into");
}
