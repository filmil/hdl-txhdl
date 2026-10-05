// SPDX-License-Identifier: Apache-2.0
//! `shadeprobe`'s list (issue 989), against Razboj's encoder and its
//! model on the host: the triangle's words are the encoder's, and the
//! pixel the program expects at each place of the screen is the one
//! the model draws there.
mod shadeprobe_list;

use razboj::dl::{decode, encode};
use razboj::model::render;
use razboj::op::{Op, SUB};
use shadeprobe_list::{expect, H, LIST, ROW, W};

/// The triangle's corners in pixels and their colours: red at the top,
/// green at the bottom right, blue at the bottom left.
const CORNERS: [(i32, i32); 3] = [(320, 40), (580, 440), (60, 440)];
const COLOURS: [u32; 3] = [0xff_00_00, 0x00_ff_00, 0x00_00_ff];

#[test]
fn the_triangle_is_the_encoders() {
    let q = |p: (i32, i32)| (p.0 * SUB, p.1 * SUB);
    let op = Op::Gouraud {
        a: q(CORNERS[0]),
        b: q(CORNERS[1]),
        c: q(CORNERS[2]),
        colours: COLOURS,
    };
    let insn = op.encode(W as usize, H as usize).unwrap();
    assert_eq!(LIST[1], encode(&insn));
}

#[test]
fn every_pixel_expected_is_the_models() {
    let ops: Vec<_> = LIST.iter().map(|w| decode(w)).collect();
    let fb = render(&ops, ROW as usize, H as usize);
    let mut shaded = 0;
    for y in 0..H {
        for x in 0..W {
            let got = fb[(y * ROW + x) as usize];
            assert_eq!(expect(x, y), got, "pixel {x},{y}");
            shaded += (got != shadeprobe_list::BACK) as u32;
        }
    }
    // As many as its area, 520 by 400 over two; `shade ok` says this
    // number on the board.
    assert_eq!(shaded, 104_000, "the pixels the triangle covers");
}

/// Each corner's own colour is reached near it, so the picture is the
/// one the board check describes.
#[test]
fn the_corners_have_their_colours() {
    let near = [(320, 50), (565, 437), (75, 437)];
    for ((x, y), want) in near.into_iter().zip(COLOURS) {
        let got = expect(x, y) & 0xff_ffff;
        for at in [16, 8, 0] {
            let (g, w) = ((got >> at) & 0xff, (want >> at) & 0xff);
            assert!(g.abs_diff(w) < 24, "{x},{y}: {got:06x} for {want:06x}");
        }
    }
}
