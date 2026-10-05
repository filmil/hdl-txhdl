// SPDX-License-Identifier: Apache-2.0
//! The icosahedron's lists (issue 986), rendered on the host through
//! Razboj's model: what the board's rasteriser draws from them, without
//! the board.
mod ico_list;

use ico_list::{frame, Box, Solid, BACKDROP, FACES, H, MOST, SECOND, W, WORDS};
use razboj::dl::decode;
use razboj::model::{coverage, render};
use razboj::op::Insn;

/// The framebuffer as the board has it: rows of 1024 pixels, and room
/// for both frames.
const FW: usize = 1024;
const FH: usize = 1024;

fn insns(list: &[[u32; WORDS]]) -> Vec<Insn> {
    list.iter().map(|w| decode(w)).collect()
}

fn list(s: &Solid, ay: i32, ax: i32, dy: i32, clear: Box) -> (Vec<Insn>, Box) {
    let mut out = [[0u32; WORDS]; MOST];
    let (n, b) = frame(s, ay, ax, dy, clear, &mut out);
    (insns(&out[..n]), b)
}

#[test]
fn the_solid_has_twenty_faces() {
    assert_eq!(Solid::new().found, FACES);
}

/// The solid stays clear of the logo's corner, so the clears never
/// touch the logo the program paints once.
#[test]
fn the_solid_cannot_reach_the_logo() {
    let logo_x = W - txhdl_logo::W as i32 - 8;
    let logo_y = H - txhdl_logo::H as i32 - 8;
    assert!(
        W / 2 + ico_list::REACH < logo_x,
        "reach {}",
        ico_list::REACH
    );
    let s = Solid::new();
    for a in 0..256 {
        let (_, b) = list(&s, a, (a * 3) & 255, 0, Box::SCREEN);
        assert!(b.x1 < logo_x || b.y1 < logo_y, "angle {a}: {b:?}");
    }
}

/// At every angle, in either frame: the faces drawn cover each pixel at
/// most once, inside the box the frame reports and inside that frame's
/// rows; and they are the faces that face the camera.
#[test]
fn the_faces_tile_the_silhouette_once_in_their_frame() {
    let s = Solid::new();
    for step in 0..64 {
        let (ay, ax) = ((step * 4) & 255, (step * 7) & 255);
        for dy in [0, SECOND] {
            let (ops, b) = list(&s, ay, ax, dy, Box::SCREEN);
            let faces = &ops[1..];
            assert!(
                (6..=14).contains(&faces.len()),
                "{} faces at {ay},{ax}",
                faces.len()
            );
            let n = coverage(faces, FW, FH);
            let mut filled = 0;
            for y in 0..FH {
                for x in 0..FW {
                    let c = n[y * FW + x];
                    assert!(c <= 1, "pixel {x},{y} drawn {c} times");
                    if c == 1 {
                        filled += 1;
                        let (x, y) = (x as i32, y as i32 - dy);
                        assert!(
                            x >= b.x0 && x <= b.x1 && y >= b.y0 && y <= b.y1,
                            "pixel {x},{y} outside {b:?}"
                        );
                    }
                }
            }
            assert!(filled > 40_000, "only {filled} pixels at {ay},{ax}");
        }
    }
}

/// A face drawn is lit at least as much as the ambient: the brightest
/// face drawn is far brighter than the ambient alone, which is what a
/// face turned away from the light would be.
#[test]
fn the_faces_drawn_are_the_front_faces() {
    let s = Solid::new();
    for step in 0..32 {
        let (ops, _) = list(&s, step * 8, step * 5, 0, Box::SCREEN);
        let brightest = ops[1..]
            .iter()
            .map(|i| (i.colour.raw() as u32 >> 16) & 0xff)
            .max()
            .unwrap();
        assert!(brightest > 0xc0, "brightest red {brightest:#x}");
    }
}

/// Lay a rendered list over a framebuffer: what the model drew
/// replaces what was there, and what it left at zero was not drawn.
fn lay(fb: &mut [u32], ops: &[Insn]) {
    let drawn = render(ops, FW, FH);
    for (p, d) in fb.iter_mut().zip(drawn) {
        if d != 0 {
            *p = d;
        }
    }
}

/// The program's loop on the host. Both frames are cleared whole once,
/// then each frame clears only the box its solid filled last time. After
/// every frame, the frame just drawn is pixel for pixel what a clear of
/// the whole screen and the same faces would give.
#[test]
fn clearing_the_last_box_leaves_the_frame_as_a_whole_clear_would() {
    let s = Solid::new();
    let mut fb = vec![0u32; FW * FH];
    let whole = |dy: i32| insns(&[ico_list::rect(BACKDROP, Box::SCREEN, dy)]);
    lay(&mut fb, &whole(0));
    lay(&mut fb, &whole(SECOND));
    let mut last = [Box::SCREEN, Box::SCREEN];
    let (mut ay, mut ax) = (0, 0);
    for k in 0..48 {
        let which = k & 1;
        let dy = which as i32 * SECOND;
        let (ops, b) = list(&s, ay, ax, dy, last[which]);
        lay(&mut fb, &ops);
        last[which] = b;
        let (fresh_ops, _) = list(&s, ay, ax, dy, Box::SCREEN);
        let mut fresh = vec![0u32; FW * FH];
        lay(&mut fresh, &whole(dy));
        lay(&mut fresh, &fresh_ops);
        for y in dy..dy + H {
            for x in 0..W {
                let i = y as usize * FW + x as usize;
                assert_eq!(fb[i], fresh[i], "frame {k}, pixel {x},{y}");
            }
        }
        ay = (ay + 2) & 255;
        ax = (ax + 1) & 255;
    }
}
