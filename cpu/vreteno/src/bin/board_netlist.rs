// SPDX-License-Identifier: Apache-2.0
//! The board's netlist, with a program in it: the Verilog of the whole
//! lowered design, the core's instruction memory and the data memory
//! holding the DDR3 test's image, on standard output.
//!
//! `sim` writes the design a simulation runs, with the serial port fast
//! and the controller's power-on waits shortened for the Micron model;
//! `board` writes the one that goes on the Alinx AX7A200, at 115200 baud
//! from the 100 MHz clock and with the controller's waits in full. The
//! instruction memory and the data memory are initialised in the
//! netlist, since nothing on the machine loads them at run time.
use txhdl::netlist::Lowered;
use vreteno32::board::Board;

/// The image, in the two memories of the core's module and the data
/// memory's: a word per instruction, and a byte per lane per word.
fn load(net: &mut Lowered, text: &[u32], data: &[u8]) {
    let text: Vec<u128> = text.iter().map(|&w| w as u128).collect();
    for inst in &mut net.instances {
        match inst.name.as_str() {
            "cpu" => inst.unit.init("imem", &text),
            "dmem" => {
                for lane in 0..4 {
                    let bytes: Vec<u128> = data
                        .chunks(4)
                        .map(|w| *w.get(lane).unwrap_or(&0) as u128)
                        .collect();
                    inst.unit.init(&format!("lane{lane}"), &bytes);
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    // The board's program says a header and a dot every ten seconds
    // around the test; a simulation's does not, since the tests state
    // the whole of what a run said and every byte is simulated time.
    let (mut net, text, data) = match mode.as_str() {
        // Sixteen cycles a bit, and the controller's simulation waits.
        "sim" => (
            Board::<16, 1, 0>::lowered("board"),
            ddr3_program::TEXT,
            ddr3_program::DATA,
        ),
        // 100 MHz over 115200 baud is 868 cycles a bit.
        "board" => (
            Board::<868, 0, 0>::lowered("board"),
            ddr3_board_program::TEXT,
            ddr3_board_program::DATA,
        ),
        _ => {
            eprintln!("usage: board_netlist sim|board");
            std::process::exit(2);
        }
    };
    load(&mut net, text, data);
    print!("{}", net.verilog());
}
