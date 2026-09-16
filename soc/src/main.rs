// SPDX-License-Identifier: Apache-2.0
//! The system, run: a core and a GPU on two corners of a lattice, a
//! memory and a serial port on the other two, all at once.
//!
//! The core runs the Rust program compiled for it and says its line
//! on the serial port; the rasteriser draws a scene into the same
//! memory, at a base above the program's data. What comes out is the
//! terminal's transcript and the picture, both read out of the one
//! memory at the end.
use gpu::{image, model, op, scene};
use soc::{run, FB_BASE, H, LOGW};

/// The screen's width.
const W: usize = 1 << LOGW;
/// What the program is written to print.
const EXPECTED: &str = "hello from rust\n";

fn main() {
    let ops = scene::house();
    let ran = run(hello_program::TEXT, hello_program::DATA, &ops, 400_000);

    if std::env::args().any(|a| a == "--ascii") {
        print!("{}", image::ascii(&ran.fb, W, H));
    } else {
        println!("{}", image::ansi(&ran.fb, W, H));
    }
    println!("the serial port said: {:?}", ran.said);
    match ran.halted_at {
        Some(c) => println!("the core halted itself at cycle {c}"),
        None => println!("the core never halted"),
    }
    match ran.drawn_at {
        Some(c) => println!("the GPU finished drawing at cycle {c}"),
        None => println!("the GPU never finished"),
    }
    println!(
        "{} entries, {W} by {H} pixels at {FB_BASE:#x}, {} cycles in all",
        ops.len(),
        ran.cycles
    );
    if let Ok(p) = std::env::var("SOC_PNG") {
        std::fs::write(&p, image::png(&ran.fb, W, H, 6)).expect("the PNG");
    }

    // Both ends did their work, and the memory holds what each of
    // them put there.
    assert_eq!(ran.said, EXPECTED, "what the core printed");
    assert!(ran.halted_at.is_some(), "the core halted itself");
    assert!(ran.drawn_at.is_some(), "the GPU finished drawing");
    let want = model::render(&op::assemble(&ops, W, H), W, H);
    let wrong = ran.fb.iter().zip(&want).filter(|(a, b)| a != b).count();
    assert_eq!(wrong, 0, "{wrong} pixels differ from the model");
    println!(
        "a core and a GPU on one lattice: the line was said and every \
         pixel is as the model says"
    );
}

/// The same run, as a test.
#[cfg(test)]
mod tests {
    use super::{EXPECTED, H, W};
    use gpu::{model, op, scene};
    use soc::run;

    #[test]
    fn a_core_and_a_gpu_share_one_lattice() {
        let ops = scene::house();
        let ran = run(hello_program::TEXT, hello_program::DATA, &ops, 400_000);
        assert_eq!(ran.said, EXPECTED, "what the core printed");
        assert!(ran.halted_at.is_some(), "the core halted itself");
        assert!(ran.drawn_at.is_some(), "the GPU finished drawing");
        let want = model::render(&op::assemble(&ops, W, H), W, H);
        assert_eq!(ran.fb, want, "the picture in the memory");
    }
}
