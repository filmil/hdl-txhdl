// SPDX-License-Identifier: Apache-2.0
//! The system, run: Vreteno and Razboj on two corners of a lattice, a
//! memory and a serial port on the other two, all at once.
//!
//! The scene is not written here. The core runs a program compiled
//! for it that works out an icosahedron, turns it to face the camera,
//! shades it and writes what faces the camera into memory as a display
//! list; the rasteriser, which has been reading the list's count
//! since the first cycle, sees it appear and fills every triangle.
//! What comes out is the terminal's transcript and the picture, both
//! read out of the one memory at the end, and the list itself, which
//! is what the core asked for and so what the picture is held to.
use razboj::sim::run_list;
use razboj::{image, model};
use soc::{run, FB_BASE, H, LOGW};

/// The screen's width.
const W: usize = 1 << LOGW;
/// The memory of the link the GPU is measured on alone: the
/// framebuffer at nought, then the list and its count above it, and
/// a power of two.
const LINK_WORDS: usize = 16384;
const LINK_DL: usize = 0xc000;
const LINK_CTRL: usize = 0xd000;
/// What the program is written to print.
const EXPECTED: &str = "icosahedron\n";

fn main() {
    let ran = run(ico_program::TEXT, ico_program::DATA, 800_000);

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
    match ran.listed_at {
        Some(c) => println!("the core finished the display list at cycle {c}"),
        None => println!("the core never finished the display list"),
    }
    match ran.drawn_at {
        Some(c) => println!("the GPU finished drawing at cycle {c}"),
        None => println!("the GPU never finished"),
    }
    println!(
        "the core wrote {} entries, {W} by {H} pixels at {FB_BASE:#x}, \
         {} cycles in all",
        ran.list.len(),
        ran.cycles
    );
    if let Ok(p) = std::env::var("SOC_PNG") {
        std::fs::write(&p, image::png(&ran.fb, W, H, 4)).expect("the PNG");
    }

    check(&ran);

    // What the lattice costs the GPU: the same list, drawn by the
    // rasteriser alone on a link with its framebuffer behind it. The
    // picture has to be the same one, and the cycles are the ones to
    // hold the lattice's against, counted from the list being ready.
    let alone = run_list::<LOGW, H, LINK_WORDS, LINK_DL, LINK_CTRL>(
        &ran.list, false, false,
    );
    assert_eq!(alone.fb, ran.fb, "the same list drew a different picture");
    if let (Some(l), Some(d)) = (ran.listed_at, ran.drawn_at) {
        println!(
            "the GPU drew the list in {} cycles on the lattice and in {} \
             on a link",
            d - l,
            alone.cycles
        );
    }
    println!(
        "a core and a GPU on one lattice: the core drew the solid and \
         every pixel of it is as the model says"
    );
}

/// What a run has to have come to. The picture is held to the display
/// list that was found in the memory afterwards, so the check is that
/// the rasteriser drew what the core asked for, whatever the core
/// asked for.
fn check(ran: &soc::Ran) {
    assert_eq!(ran.said, EXPECTED, "what the core printed");
    assert!(ran.halted_at.is_some(), "the core halted itself");
    assert!(ran.drawn_at.is_some(), "the GPU finished drawing");
    // An icosahedron shows about half of its twenty faces, and the list
    // begins with the space behind it.
    assert!(
        ran.list.len() >= 8,
        "the core wrote only {} entries",
        ran.list.len()
    );
    let want = model::render(&ran.list, W, H);
    let wrong = ran.fb.iter().zip(&want).filter(|(a, b)| a != b).count();
    assert_eq!(wrong, 0, "{wrong} pixels differ from the model");
    // The solid is drawn, not merely cleared to: some of the screen is
    // neither the space colour nor black.
    let space = ran.fb[0];
    let solid = ran.fb.iter().filter(|&&p| p != space).count();
    assert!(solid > 400, "only {solid} pixels of the solid");
}

/// The same run, as a test.
#[cfg(test)]
mod tests {
    use super::check;
    use soc::run;

    #[test]
    fn a_core_draws_a_solid_and_a_gpu_fills_it() {
        let ran = run(ico_program::TEXT, ico_program::DATA, 800_000);
        check(&ran);
    }
}
