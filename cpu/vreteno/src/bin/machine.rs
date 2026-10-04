// SPDX-License-Identifier: Apache-2.0
//! The Vreteno board as a fast machine, from the command line (issue
//! 1016): a flat image and a device tree blob loaded into the DDR3, the
//! hart started at the image, and the serial port's output on standard
//! output as it is sent.
//!
//! ```text
//! bazel run //cpu/vreteno:machine -- --image $PWD/fw_jump.bin \
//!     [--at 0x40000000] [--dtb $PWD/vreteno.dtb] [--dtb-at 0x41008000] \
//!     [--steps 100000000]
//! ```
//!
//! It stops when the hart halts, or after `--steps` instructions, and
//! says which, with the program counter, on standard error.
use std::io::Write;
use vreteno32::machine::Machine;

/// Where the image goes and starts, by default: the DDR3's base, which
/// is OpenSBI's `FW_TEXT_START`.
const AT: u32 = 0x4000_0000;
/// Where the blob goes by default: above the Ethernet buffers, clear of
/// OpenSBI's jump address and of where it moves the blob to.
const DTB_AT: u32 = 0x4100_8000;

fn number(s: &str) -> u64 {
    let s = s.replace('_', "");
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16),
        None => s.parse(),
    }
    .unwrap_or_else(|_| panic!("not a number: {s}"))
}

fn main() {
    let mut image = None;
    let mut at = AT;
    let mut dtb = None;
    let mut dtb_at = DTB_AT;
    let mut steps = 100_000_000u64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val =
            || args.next().unwrap_or_else(|| panic!("{a} wants a value"));
        match a.as_str() {
            "--image" => image = Some(val()),
            "--at" => at = number(&val()) as u32,
            "--dtb" => dtb = Some(val()),
            "--dtb-at" => dtb_at = number(&val()) as u32,
            "--steps" => steps = number(&val()),
            _ => panic!("unknown argument {a}"),
        }
    }
    let image = image.expect("--image is required");
    let bytes =
        std::fs::read(&image).unwrap_or_else(|e| panic!("{image}: {e}"));
    let mut m = Machine::new();
    m.load(at, &bytes);
    if let Some(d) = &dtb {
        let blob = std::fs::read(d).unwrap_or_else(|e| panic!("{d}: {e}"));
        m.load(dtb_at, &blob);
    }
    m.boot(at, if dtb.is_some() { dtb_at } else { 0 });
    let out = std::io::stdout();
    let mut out = out.lock();
    let mut shown = 0;
    let mut ran = 0u64;
    // A slice at a time, so the output streams without a check every
    // instruction.
    while ran < steps && m.model.halted.is_none() {
        ran += m.run((steps - ran).min(100_000));
        let sent = &m.board.0.borrow().uart.sent;
        if sent.len() > shown {
            out.write_all(&sent[shown..]).ok();
            out.flush().ok();
            shown = sent.len();
        }
    }
    let how = match m.model.halted {
        Some(h) => format!("halted ({h:?})"),
        None => "stopped at the step limit".to_string(),
    };
    eprintln!("\n{how} after {ran} instructions, pc {:#010x}", m.model.pc);
}
