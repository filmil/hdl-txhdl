// SPDX-License-Identifier: Apache-2.0
//! The Vreteno board as a fast machine, from the command line (issue
//! 1016): a flat image and a device tree blob loaded into the DDR3, the
//! hart started at the image, the serial port's output on standard
//! output as it is sent, and standard input on the port's receive side
//! as it arrives (issue 1127), so a shell on the machine can be typed
//! into or fed from a pipe.
//!
//! ```text
//! bazel run //cpu/vreteno:machine -- --image $PWD/fw_jump.bin \
//!     [--at 0x40000000] [--dtb $PWD/vreteno.dtb] [--dtb-at 0x41008000] \
//!     [--steps 100000000] [--as-loaded BYTES]
//! ```
//!
//! `--as-loaded` starts the serial port as the serial loader leaves it
//! on the board (issue 1136): its receive interrupt enabled, as the
//! hardware resets it, and `BYTES` waiting to be read, so the line is
//! high and the interrupt controller has the request before the image
//! runs.
//!
//! It stops when the hart halts, or after `--steps` instructions, and
//! says which, with the program counter, on standard error.
use std::io::{Read, Write};
use std::sync::mpsc;
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
    let mut loaded = None;
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
            "--as-loaded" => loaded = Some(val()),
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
    if let Some(bytes) = &loaded {
        m.board.0.borrow_mut().uart.ie = 2;
        m.type_bytes(bytes.as_bytes());
    }
    let out = std::io::stdout();
    let mut out = out.lock();
    let mut shown = 0;
    let mut ran = 0u64;
    // A slice at a time, so the output streams without a check every
    // instruction.
    // Standard input, read on a thread of its own so that a read that
    // waits for a terminal never stops the machine; what has arrived is
    // handed to the port between slices.
    let (typed_tx, typed) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 256];
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 || typed_tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    while ran < steps && m.model.halted.is_none() {
        while let Ok(bytes) = typed.try_recv() {
            m.type_bytes(&bytes);
        }
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
