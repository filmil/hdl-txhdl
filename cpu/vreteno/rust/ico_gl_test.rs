// SPDX-License-Identifier: Apache-2.0
//! The icosahedron through the GL ES library against the one written by
//! hand (issue 995), both drawn on the host through Razboj's model: the
//! check `docs/gles.md` section 8 sets for "the same picture".
//!
//! At every angle tried, in either frame: the same number of faces, in
//! the same order, each within 2 of the other's colour in every
//! channel; and every pixel where the two pictures differ by more than
//! that lies on an edge in both, one pixel from a pixel of another
//! colour.
mod ico_gl;
mod ico_list;

use ico_gl::Model;
use ico_list::{Box, Solid, H, MOST, SECOND, W, WORDS};
use razboj::dl::decode;
use razboj::model::render;
use razboj::op::Insn;

/// The framebuffer as the board has it.
const FW: usize = 1024;
const FH: usize = 1024;

fn insns(list: &[[u32; WORDS]]) -> Vec<Insn> {
    list.iter().map(|w| decode(w)).collect()
}

/// A colour's three channels.
fn rgb(c: u32) -> [i32; 3] {
    [
        ((c >> 16) & 0xff) as i32,
        ((c >> 8) & 0xff) as i32,
        (c & 0xff) as i32,
    ]
}

/// Whether two colours are within `d` of each other in every channel.
fn near(a: u32, b: u32, d: i32) -> bool {
    rgb(a).iter().zip(rgb(b)).all(|(x, y)| (x - y).abs() <= d)
}

/// Whether the pixel at `x`, `y` has a neighbour of another colour.
fn on_edge(fb: &[u32], x: usize, y: usize) -> bool {
    let c = fb[y * FW + x] & 0xff_ffff;
    (y.saturating_sub(1)..=(y + 1).min(FH - 1)).any(|v| {
        (x.saturating_sub(1)..=(x + 1).min(FW - 1))
            .any(|u| fb[v * FW + u] & 0xff_ffff != c)
    })
}

/// Both lists for one frame, each cleared whole first.
fn lists(
    s: &Solid,
    m: &Model,
    ay: i32,
    ax: i32,
    dy: i32,
) -> (Vec<Insn>, Vec<Insn>) {
    let mut out = [[0u32; WORDS]; MOST];
    let (n, _) = ico_list::frame(s, ay, ax, dy, Box::SCREEN, &mut out);
    let hand = insns(&out[..n]);
    let mut out = [[0u32; WORDS]; MOST];
    let (n, _) = ico_gl::frame(m, ay, ax, dy, Box::SCREEN, &mut out);
    (hand, insns(&out[..n]))
}

#[test]
fn the_gl_icosahedron_draws_the_same_picture() {
    let s = Solid::new();
    let m = Model::new(&s);
    let mut worst = 0;
    let mut edges = 0usize;
    for step in 0..64 {
        let (ay, ax) = ((step * 4) & 255, (step * 7) & 255);
        for dy in [0, SECOND] {
            let (hand, gl) = lists(&s, &m, ay, ax, dy);
            assert_eq!(
                hand.len(),
                gl.len(),
                "faces at {ay},{ax}: hand {} gl {}",
                hand.len() - 1,
                gl.len() - 1
            );
            for (k, (h, g)) in hand.iter().zip(&gl).enumerate().skip(1) {
                let (h, g) = (h.colour.raw() as u32, g.colour.raw() as u32);
                for (a, b) in rgb(h).iter().zip(rgb(g)) {
                    worst = worst.max((a - b).abs());
                }
                assert!(
                    near(h, g, 2),
                    "face {k} at {ay},{ax}: hand {h:06x} gl {g:06x}"
                );
            }
            let (fh, fg) = (render(&hand, FW, FH), render(&gl, FW, FH));
            for y in dy as usize..(dy + H) as usize {
                for x in 0..W as usize {
                    let (a, b) = (fh[y * FW + x], fg[y * FW + x]);
                    if near(a, b, 2) {
                        continue;
                    }
                    edges += 1;
                    assert!(
                        on_edge(&fh, x, y) && on_edge(&fg, x, y),
                        "pixel {x},{y} at {ay},{ax}: hand {a:06x} gl {b:06x}"
                    );
                }
            }
        }
    }
    println!("worst channel {worst}, {edges} edge pixels differ");
}

/// The GL frame's box covers what it draws, so that clearing it next
/// time clears every pixel of the solid, as `ico_list`'s does.
#[test]
fn the_gl_frame_box_covers_its_faces() {
    let s = Solid::new();
    let m = Model::new(&s);
    for step in 0..64 {
        let (ay, ax) = ((step * 4) & 255, (step * 7) & 255);
        for dy in [0, SECOND] {
            let mut out = [[0u32; WORDS]; MOST];
            let (n, b) = ico_gl::frame(&m, ay, ax, dy, Box::SCREEN, &mut out);
            let drawn = render(&insns(&out[1..n]), FW, FH);
            for y in 0..FH {
                for x in 0..FW {
                    if drawn[y * FW + x] != 0 {
                        let (x, y) = (x as i32, y as i32 - dy);
                        assert!(
                            x >= b.x0 && x <= b.x1 && y >= b.y0 && y <= b.y1,
                            "pixel {x},{y} outside {b:?} at {ay},{ax}"
                        );
                    }
                }
            }
            // No nearer the logo's corner than `ico_list` reaches.
            let reach = ico_list::REACH;
            assert!(
                b.x1 <= W / 2 + reach && b.y1 <= H / 2 + reach,
                "{b:?} past the reach {reach} at {ay},{ax}"
            );
        }
    }
}
