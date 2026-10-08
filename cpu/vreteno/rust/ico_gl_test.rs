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
// Shared with the board program and ico_tex_frame, which use parts of
// it these tests do not.
#[allow(dead_code)]
mod ico_gl;
mod ico_list;

use ico_gl::Model;
use ico_list::{Box, Solid, H, SECOND, W, WORDS};
use razboj::dl::decode_list;
use razboj::model::render;
use razboj::op::Insn;

/// The framebuffer as the board has it.
const FW: usize = 1024;
const FH: usize = 1024;

fn insns(list: &[[u32; WORDS]]) -> Vec<Insn> {
    decode_list(list)
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
    let mut out = [[0u32; WORDS]; ico_list::MOST];
    let (n, _) = ico_list::frame(s, ay, ax, dy, Box::SCREEN, &mut out);
    let hand = insns(&out[..n]);
    let mut out = [[0u32; WORDS]; ico_gl::MOST];
    let (n, _) =
        ico_gl::frame(m, ay, ax, dy, Box::SCREEN, false, None, &mut out);
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

/// With the depth test instead of culling (#1273), the picture is the
/// culled one: every face is drawn, and each pixel is the nearest
/// face's. The list tests depth, so it is a tile table's to draw, and
/// the model draws it as one draws it. A pixel may differ only on the
/// solid's outline, where a back face meets a front one at one depth
/// and rounding decides which is nearer.
#[test]
fn the_depth_test_draws_the_culled_picture() {
    let s = Solid::new();
    let m = Model::new(&s);
    let mut outline = 0usize;
    for step in 0..64 {
        let (ay, ax) = ((step * 4) & 255, (step * 7) & 255);
        for dy in [0, SECOND] {
            let draw = |depth: bool| {
                let mut out = [[0u32; WORDS]; ico_gl::MOST];
                let (n, _) = ico_gl::frame(
                    &m,
                    ay,
                    ax,
                    dy,
                    Box::SCREEN,
                    depth,
                    None,
                    &mut out,
                );
                let deep = out[..n].iter().any(|w| (w[15] >> 8) & 1 == 1);
                assert_eq!(deep, depth, "depth tested only when asked");
                render(&insns(&out[..n]), FW, FH)
            };
            let (culled, deep) = (draw(false), draw(true));
            for y in dy as usize..(dy + H) as usize {
                for x in 0..W as usize {
                    let (a, b) = (culled[y * FW + x], deep[y * FW + x]);
                    if a == b {
                        continue;
                    }
                    outline += 1;
                    assert!(
                        on_edge(&culled, x, y) && on_edge(&deep, x, y),
                        "pixel {x},{y} at {ay},{ax}: culled {a:06x} depth {b:06x}"
                    );
                }
            }
        }
    }
    println!("{outline} pixels on the outline differ");
}

/// The GL frame's box covers what it draws, culled or with depth, so
/// that clearing it next time clears every pixel of the solid, as
/// `ico_list`'s does.
#[test]
fn the_gl_frame_box_covers_its_faces() {
    let s = Solid::new();
    let m = Model::new(&s);
    for step in 0..64 {
        let (ay, ax) = ((step * 4) & 255, (step * 7) & 255);
        for (dy, depth) in
            [(0, false), (SECOND, false), (0, true), (SECOND, true)]
        {
            let mut out = [[0u32; WORDS]; ico_gl::MOST];
            let (n, b) = ico_gl::frame(
                &m,
                ay,
                ax,
                dy,
                Box::SCREEN,
                depth,
                None,
                &mut out,
            );
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

/// The textured frame (#997) drawn by the rasteriser itself, through the
/// AXI link into a framebuffer of the board's rows of 1024, as a tile
/// table of the board's 640 by 480 with the texture in the same memory,
/// is the model's, byte for byte: what the board will show.
#[test]
fn the_textured_frame_on_the_rasteriser_is_the_models() {
    use razboj::model::{render_textured, Textures};
    use razboj::sim::{run_works_at, Work};
    const A: usize = 24;
    const LOGW: usize = 10;
    const RH: usize = 480;
    const N: usize = 1 << 20;
    const DL: usize = 0x20_0000;
    const CTRL: usize = 0x3f_fffc;
    const TEX: u32 = 0x38_0000;
    let m = Model::new(&Solid::new());
    let mut room = vec![0u32; ico_gl::TEX_ROOM];
    let mut out = [[0u32; WORDS]; ico_gl::MOST];
    let (n, _) = ico_gl::frame(
        &m,
        80,
        40,
        0,
        Box::SCREEN,
        true,
        Some((&mut room, TEX, true)),
        &mut out,
    );
    let list = insns(&out[..n]);
    assert_eq!(list.iter().filter(|i| i.tex.to_bool()).count(), 20);
    let more: Vec<(usize, u32)> = room
        .iter()
        .enumerate()
        .map(|(k, w)| (TEX as usize + 4 * k, *w))
        .collect();
    let work = Work {
        more,
        ..Work::tiled(&list, W as usize, RH)
    };
    let runs = run_works_at::<A, LOGW, RH, N, DL, CTRL>(&[work], false, false);
    let read = |a: u32| room[((a - TEX) / 4) as usize];
    let t = Textures { mem: &read };
    let want = render_textured(&list, FW, RH, vec![0; FW * RH], Some(&t));
    let at = want.iter().zip(&runs[0].fb).position(|(p, q)| p != q);
    assert_eq!(
        at,
        None,
        "pixel {:?}: {:08x} not {:08x}",
        at.map(|a| (a % FW, a / FW)),
        at.map_or(0, |a| runs[0].fb[a]),
        at.map_or(0, |a| want[a])
    );
    println!(
        "{} cycles, {} read bursts, {} slots",
        runs[0].cycles, runs[0].reads.0, n
    );
}

/// A frame that finds the texture an earlier frame uploaded (#1433)
/// writes the same list, word for word, as one that uploads it, at every
/// angle tried and in either buffer.
#[test]
fn a_kept_texture_draws_as_an_uploaded_one() {
    const TEX: u32 = 0x38_0000;
    let m = Model::new(&Solid::new());
    let mut room = vec![0u32; ico_gl::TEX_ROOM];
    for step in 0..16 {
        let (ay, ax) = ((step * 16) & 255, (step * 9) & 255);
        let dy = (step & 1) * SECOND;
        let mut up = [[0u32; WORDS]; ico_gl::MOST];
        let mut kept = [[0u32; WORDS]; ico_gl::MOST];
        let tex = Some((&mut room[..], TEX, true));
        let (nu, _) =
            ico_gl::frame(&m, ay, ax, dy, Box::SCREEN, true, tex, &mut up);
        let tex = Some((&mut room[..], TEX, false));
        let (nk, _) =
            ico_gl::frame(&m, ay, ax, dy, Box::SCREEN, true, tex, &mut kept);
        assert_eq!(nu, nk, "slots at {ay},{ax}");
        assert!(up[..nu] == kept[..nk], "the lists differ at {ay},{ax}");
    }
}
