// SPDX-License-Identifier: Apache-2.0
//! Blending, the alpha test and the colour mask through the GL library
//! (#993), against floating point, through Razboj's model.
//!
//! Scenes of flat triangles that overlap, drawn over a backdrop, under
//! every pair of blend factors GL ES 1.1 allows, with the alpha test and
//! the colour mask: the library's frame, drawn by the model as a tile
//! table draws it, is held pixel by pixel to a reference that blends in
//! `f64`. A pixel near a triangle's edge is left out, where the fill rule
//! and the sixteenths decide.
use gles::fixed::{Fx, ONE};
use gles::{colour_word, gl, Gl};
use razboj::dl::decode_list;
use razboj::model::render_over;
use razboj_tile::WORDS;

const W: u32 = 96;
const H: u32 = 64;

fn fx(v: f64) -> Fx {
    (v * 65536.0).round() as Fx
}

/// Pseudorandom numbers from a literal seed.
struct Rng(u32);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    fn real(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * (self.next() % 100_001) as f64 / 100_000.0
    }
}

/// The factors GL ES 1.1 allows the source and the destination.
const SRC: [u32; 9] = [
    gl::ZERO,
    gl::ONE,
    gl::DST_COLOR,
    gl::ONE_MINUS_DST_COLOR,
    gl::SRC_ALPHA,
    gl::ONE_MINUS_SRC_ALPHA,
    gl::DST_ALPHA,
    gl::ONE_MINUS_DST_ALPHA,
    gl::SRC_ALPHA_SATURATE,
];
const DST: [u32; 8] = [
    gl::ZERO,
    gl::ONE,
    gl::SRC_COLOR,
    gl::ONE_MINUS_SRC_COLOR,
    gl::SRC_ALPHA,
    gl::ONE_MINUS_SRC_ALPHA,
    gl::DST_ALPHA,
    gl::ONE_MINUS_DST_ALPHA,
];

/// A triangle in GL's window, its corners in pixels with GL's y upwards
/// and its colour.
type Tri = ([(f64, f64); 3], [f64; 4]);

/// A scene's state and triangles.
struct Scene {
    tris: Vec<Tri>,
    src: u32,
    dst: u32,
    alpha: Option<(u32, f64)>,
    mask: [bool; 4],
}

/// The backdrop: bands of colour whose alpha varies, so that a factor of
/// the destination's alpha has something to read.
fn backdrop() -> Vec<u32> {
    (0..(W * H))
        .map(|i| {
            let (x, y) = (i % W, i / W);
            0x0010_2030u32
                .wrapping_add(x * 0x0002_0300)
                .wrapping_add(y * 0x0401_0004)
                | ((x * 5 + y * 3) & 0xff) << 24
        })
        .collect()
}

/// The library's frame of a scene: the triangles under its state, in an
/// orthographic window of a pixel a unit.
fn library(s: &Scene, frame: &mut [[u32; WORDS]]) -> usize {
    let mut g = Gl::new(frame, W, H);
    g.matrix_mode(gl::PROJECTION);
    g.ortho(0, (W as i32) << 16, 0, (H as i32) << 16, -ONE, ONE);
    g.matrix_mode(gl::MODELVIEW);
    g.enable(gl::BLEND);
    g.blend_func(s.src, s.dst);
    if let Some((func, r)) = s.alpha {
        g.enable(gl::ALPHA_TEST);
        g.alpha_func(func, fx(r));
    }
    let [r, gr, b, a] = s.mask;
    g.color_mask(r, gr, b, a);
    g.shade_model(gl::FLAT);
    for (v, c) in &s.tris {
        let p = v.map(|(x, y)| [fx(x), fx(y), 0, ONE]);
        let col = c.map(fx);
        g.draw_arrays(gl::TRIANGLES, &p, Some(&[col; 3]), None);
    }
    assert_eq!(g.get_error(), gl::NO_ERROR);
    assert!(g.tiled(), "a frame that blends is a tile table");
    g.frame().len()
}

/// One factor of GL's, in `f64` from nought to one, for the channel `c`
/// of the source `s` and the destination `d`, `c` 3 for alpha.
fn factor(f: u32, s: [f64; 4], d: [f64; 4], c: usize) -> f64 {
    match f {
        gl::ZERO => 0.0,
        gl::ONE => 1.0,
        gl::SRC_COLOR => s[c],
        gl::ONE_MINUS_SRC_COLOR => 1.0 - s[c],
        gl::DST_COLOR => d[c],
        gl::ONE_MINUS_DST_COLOR => 1.0 - d[c],
        gl::SRC_ALPHA => s[3],
        gl::ONE_MINUS_SRC_ALPHA => 1.0 - s[3],
        gl::DST_ALPHA => d[3],
        gl::ONE_MINUS_DST_ALPHA => 1.0 - d[3],
        _ if c == 3 => 1.0,
        _ => s[3].min(1.0 - d[3]),
    }
}

/// A word `0xAARRGGBB` as four channels from nought to one, red first.
fn channels(p: u32) -> [f64; 4] {
    [16, 8, 0, 24].map(|at| ((p >> at) & 0xff) as f64 / 255.0)
}

/// What the reference says of a pixel, over the backdrop's `under`: its
/// colour, or that it cannot say.
fn reference(s: &Scene, x: u32, y: u32, under: u32) -> Option<u32> {
    let (px, py) = (x as f64 + 0.5, (H - y) as f64 - 0.5);
    let mut out = under;
    for (v, c) in &s.tris {
        let [a, b, cc] = *v;
        let area = (b.0 - a.0) * (cc.1 - a.1) - (b.1 - a.1) * (cc.0 - a.0);
        let edge = |p: (f64, f64), q: (f64, f64)| {
            let e = (q.0 - p.0) * (py - p.1) - (q.1 - p.1) * (px - p.0);
            e * area.signum() / (q.0 - p.0).hypot(q.1 - p.1)
        };
        let d = [edge(b, cc), edge(cc, a), edge(a, b)];
        if area.abs() < 1e-9 || d.iter().any(|&e| e < -0.75) {
            continue;
        }
        if d.iter().any(|&e| e < 0.75) {
            return None;
        }
        // The source as the library rounds it from its 16.16.
        let src = colour_word(&c.map(fx));
        let sa = (src >> 24) as f64;
        if let Some((func, r)) = s.alpha {
            let r =
                ((fx(r).clamp(0, ONE) as i64 * 255 + (1 << 15)) >> 16) as f64;
            let pass = match func {
                gl::NEVER => false,
                gl::LESS => sa < r,
                gl::EQUAL => sa == r,
                gl::LEQUAL => sa <= r,
                gl::GREATER => sa > r,
                gl::NOTEQUAL => sa != r,
                gl::GEQUAL => sa >= r,
                _ => true,
            };
            if !pass {
                continue;
            }
        }
        let (sc, dc) = (channels(src), channels(out));
        let blended: [u32; 4] = std::array::from_fn(|k| {
            let v = sc[k] * factor(s.src, sc, dc, k)
                + dc[k] * factor(s.dst, sc, dc, k);
            (v * 255.0).round().min(255.0) as u32
        });
        let [r, g, b, a] = s.mask;
        let pick = |on: bool, new: u32, at: u32| {
            if on {
                new << at
            } else {
                out & (0xff << at)
            }
        };
        out = pick(r, blended[0], 16)
            | pick(g, blended[1], 8)
            | pick(b, blended[2], 0)
            | pick(a, blended[3], 24);
    }
    Some(out)
}

/// Every pair of factors GL ES 1.1 allows, a scene each, with the alpha
/// test and the colour mask on in some: the picture is the reference's
/// wherever the reference can say, and the blend changed it.
#[test]
fn blending_is_floating_points() {
    let mut r = Rng(0xb1e7_d993);
    let mut frame = vec![[0u32; WORDS]; 256];
    let under = backdrop();
    let (mut compared, mut changed) = (0, 0);
    let funcs = [gl::LESS, gl::GEQUAL, gl::NOTEQUAL, gl::ALWAYS];
    for (k, (&src, &dst)) in SRC
        .iter()
        .flat_map(|s| DST.iter().map(move |d| (s, d)))
        .enumerate()
    {
        let tris = (0..4)
            .map(|_| {
                let v = |r: &mut Rng| {
                    (r.real(-8.0, W as f64 + 8.0), r.real(-8.0, H as f64 + 8.0))
                };
                let c = [0, 1, 2, 3].map(|_| r.real(0.0, 1.0));
                ([v(&mut r), v(&mut r), v(&mut r)], c)
            })
            .collect();
        let s = Scene {
            tris,
            src,
            dst,
            alpha: (k % 3 == 1).then(|| (funcs[k % 4], r.real(0.2, 0.8))),
            mask: if k % 5 == 2 {
                [true, false, true, true]
            } else {
                [true; 4]
            },
        };
        let n = library(&s, &mut frame);
        let got = render_over(
            &decode_list(&frame[..n]),
            W as usize,
            H as usize,
            under.clone(),
        );
        for y in 0..H {
            for x in 0..W {
                let at = (y * W + x) as usize;
                let Some(want) = reference(&s, x, y, under[at]) else {
                    continue;
                };
                assert_eq!(
                    got[at], want,
                    "factors {src:04x} {dst:04x}, pixel ({x}, {y}): \
                     {:08x}, not {want:08x}",
                    got[at]
                );
                compared += 1;
                changed += (want != under[at]) as u32;
            }
        }
    }
    let all = SRC.len() as u32 * DST.len() as u32 * W * H;
    assert!(
        compared > all * 3 / 4 && changed > all / 5,
        "{compared} compared, {changed} changed of {all}"
    );
}

/// A factor GL ES 1.1 does not allow is `GL_INVALID_ENUM`, and changes
/// nothing; and the alpha test's and the mask's state reach the list.
#[test]
fn a_factor_not_allowed_is_refused() {
    let mut frame = vec![[0u32; WORDS]; 8];
    let mut g = Gl::new(&mut frame, W, H);
    g.blend_func(gl::SRC_COLOR, gl::ZERO);
    assert_eq!(g.get_error(), gl::INVALID_ENUM);
    g.blend_func(gl::ONE, gl::SRC_ALPHA_SATURATE);
    assert_eq!(g.get_error(), gl::INVALID_ENUM);
    g.blend_func(gl::ONE, gl::DST_COLOR);
    assert_eq!(g.get_error(), gl::INVALID_ENUM);
    g.alpha_func(gl::ALWAYS + 1, 0);
    assert_eq!(g.get_error(), gl::INVALID_ENUM);
}

/// A clear of the depth alone (#993, which lifts #1273's limit): after a
/// near triangle, the depth cleared to the farthest lets a far one drawn
/// after it through, and the near one's colour stays where the far one
/// does not reach.
#[test]
fn a_depth_clear_alone_clears_only_the_depth() {
    let mut frame = vec![[0u32; WORDS]; 16];
    let mut g = Gl::new(&mut frame, W, H);
    g.matrix_mode(gl::PROJECTION);
    g.ortho(0, (W as i32) << 16, 0, (H as i32) << 16, -ONE, ONE);
    g.matrix_mode(gl::MODELVIEW);
    g.enable(gl::DEPTH_TEST);
    g.clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
    let tri = |x0: f64, z: f64| {
        [(x0, 4.0), (x0 + 60.0, 4.0), (x0, 60.0)]
            .map(|(x, y)| [fx(x), fx(y), fx(z), ONE])
    };
    g.color(ONE, 0, 0, ONE);
    g.draw_arrays(gl::TRIANGLES, &tri(4.0, 0.5), None, None);
    g.clear(gl::DEPTH_BUFFER_BIT);
    g.color(0, ONE, 0, ONE);
    g.draw_arrays(gl::TRIANGLES, &tri(20.0, -0.5), None, None);
    let n = g.frame().len();
    let fb = render_over(
        &decode_list(&frame[..n]),
        W as usize,
        H as usize,
        vec![0; (W * H) as usize],
    );
    let at = |x: u32, y: u32| fb[((H - 1 - y) * W + x) as usize];
    assert_eq!(
        at(30, 10),
        0xff00_ff00,
        "the far one, drawn after the clear"
    );
    assert_eq!(at(8, 10), 0xffff_0000, "the near one, outside the far one");
}
