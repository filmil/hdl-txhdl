// SPDX-License-Identifier: Apache-2.0
//! Depth through the GL library (#1273), against floating point, through
//! Razboj's model.
//!
//! Scenes of triangles that overlap and cross in depth, in perspective,
//! under each comparison, with the depth written and not, over a clear of
//! the depth and inside a depth range: the library's frame, drawn by the
//! model as a tile table draws it, is held pixel by pixel to a reference
//! that does the same arithmetic in `f64` and decides each pixel by the
//! nearest triangle there. A pixel is left out where the reference cannot
//! say for certain: near a triangle's edge, where the fill rule and the
//! sixteenths decide, and where two depths there lie within two units of
//! each other, where rounding decides.
use gles::fixed::{Fx, ONE};
use gles::{colour_word, gl, Gl};
use razboj::dl::decode_list;
use razboj::model::render;
use razboj_tile::{TILE_WORDS, WORDS};

const W: u32 = 160;
const H: u32 = 120;

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
    /// A number from `lo` to `hi`.
    fn real(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * (self.next() % 100_001) as f64 / 100_000.0
    }
}

/// A scene: its triangles in eye coordinates, each with its colour, and
/// the depth state it is drawn under.
struct Scene {
    tris: Vec<([[f64; 3]; 3], [f64; 3])>,
    func: u32,
    mask: bool,
    clear_depth: f64,
    range: (f64, f64),
}

/// The frustum every scene is seen through: a near plane one unit away,
/// a far one ten, and the screen's shape.
const FRUSTUM: [f64; 6] = [-1.0, 1.0, -0.75, 0.75, 1.0, 10.0];

/// A scene of `n` triangles, each well inside the frustum, so that none
/// is clipped, and each large enough to overlap others.
fn scene(r: &mut Rng, n: usize) -> Scene {
    let tris = (0..n)
        .map(|_| {
            let v = |r: &mut Rng| {
                let z = r.real(-9.0, -1.5);
                [r.real(-0.9, 0.9) * -z, r.real(-0.7, 0.7) * -z, z]
            };
            let c = [r.real(0.0, 1.0), r.real(0.0, 1.0), r.real(0.0, 1.0)];
            ([v(r), v(r), v(r)], c)
        })
        .collect();
    let funcs = [gl::LESS, gl::LEQUAL, gl::GREATER, gl::GEQUAL, gl::ALWAYS];
    let func = funcs[r.next() as usize % funcs.len()];
    // A comparison that keeps the farther needs the clear nearer.
    let far = matches!(func, gl::GREATER | gl::GEQUAL);
    let near = r.real(0.0, 0.3);
    Scene {
        tris,
        func,
        mask: !r.next().is_multiple_of(4),
        clear_depth: if far { 0.0 } else { r.real(0.7, 1.0) },
        range: (near, r.real(0.7, 1.0)),
    }
}

/// The library's frame of a scene: a clear of the colour and the depth,
/// then the triangles, flat, with the depth test on.
fn library(s: &Scene, frame: &mut [[u32; WORDS]]) -> usize {
    let mut g = Gl::new(frame, W, H);
    let [l, r, b, t, n, f] = FRUSTUM.map(fx);
    g.matrix_mode(gl::PROJECTION);
    g.frustum(l, r, b, t, n, f);
    g.matrix_mode(gl::MODELVIEW);
    g.enable(gl::DEPTH_TEST);
    g.depth_func(s.func);
    g.depth_mask(s.mask);
    g.clear_depth(fx(s.clear_depth));
    g.depth_range(fx(s.range.0), fx(s.range.1));
    g.clear_color(0, 0, 0, ONE);
    g.clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
    g.shade_model(gl::FLAT);
    for (v, c) in &s.tris {
        let p = v.map(|v| [fx(v[0]), fx(v[1]), fx(v[2]), ONE]);
        let col = [fx(c[0]), fx(c[1]), fx(c[2]), ONE];
        g.draw_arrays(gl::TRIANGLES, &p, Some(&[col; 3]), None);
    }
    assert_eq!(g.get_error(), gl::NO_ERROR);
    assert!(g.tiled(), "a frame that tests depth is a tile table");
    g.frame().len()
}

/// A point in eye coordinates in Razboj's window: x and y in pixels, y
/// downwards, snapped to the sixteenth as the library snaps a vertex, and
/// the window depth in units of sixteen bits, rounded as the library
/// rounds a vertex's.
fn window(s: &Scene, v: [f64; 3]) -> (f64, f64, f64) {
    let [l, r, b, t, n, f] = FRUSTUM;
    let w = -v[2];
    let xn = (2.0 * n * v[0] / (r - l) + (r + l) / (r - l) * v[2]) / w;
    let yn = (2.0 * n * v[1] / (t - b) + (t + b) / (t - b) * v[2]) / w;
    let zn = (-(f + n) / (f - n) * v[2] - 2.0 * f * n / (f - n)) / w;
    let x = W as f64 * (xn + 1.0) / 2.0;
    let y = H as f64 - H as f64 * (yn + 1.0) / 2.0;
    let (dn, df) = s.range;
    let snap = |v: f64| (v * 16.0).round() / 16.0;
    let z = ((dn + (df - dn) * (zn + 1.0) / 2.0) * 65535.0).round();
    (snap(x), snap(y), z)
}

/// What the reference says of a pixel: its colour, or that it cannot
/// say, and whether a triangle there lost a depth test.
fn reference(s: &Scene, x: usize, y: usize) -> (Option<u32>, bool) {
    let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
    let mut colour = 0xff00_0000u32;
    // The clear writes the depth only where the mask lets it (#993); a
    // tile's depth is the farthest until something writes it.
    let mut depth = if s.mask {
        (s.clear_depth * 65535.0).round()
    } else {
        65535.0
    };
    // How far the depth there may be from the library's: a unit for the
    // rounding of the planes, and what the depth moves across a
    // sixteenth of a pixel, where a vertex worked out in fixed point may
    // land instead.
    let mut slack = 1.0;
    let mut hidden = false;
    for (v, c) in &s.tris {
        let [a, b, cc] = v.map(|v| window(s, v));
        let area = (b.0 - a.0) * (cc.1 - a.1) - (b.1 - a.1) * (cc.0 - a.0);
        // Each edge's distance from the pixel's centre, in pixels, signed
        // so that inside is positive whichever way the triangle winds.
        let edge = |p: (f64, f64, f64), q: (f64, f64, f64)| {
            let e = (q.0 - p.0) * (py - p.1) - (q.1 - p.1) * (px - p.0);
            e.signum() * area.signum() * e.abs()
                / ((q.0 - p.0).hypot(q.1 - p.1))
        };
        let d = [edge(b, cc), edge(cc, a), edge(a, b)];
        if area.abs() < 1e-9 || d.iter().all(|&e| e < -0.75) {
            continue;
        }
        if d.iter().any(|&e| e.abs() < 0.75) {
            return (None, false);
        }
        if d.iter().any(|&e| e < 0.0) {
            continue;
        }
        // Inside: the depth, linear across the window.
        let wa = d[0] * ((b.0 - cc.0).hypot(b.1 - cc.1));
        let wb = d[1] * ((cc.0 - a.0).hypot(cc.1 - a.1));
        let wc = d[2] * ((a.0 - b.0).hypot(a.1 - b.1));
        let z = (wa * a.2 + wb * b.2 + wc * cc.2) / (wa + wb + wc);
        let (ab, ac) = ((b.0 - a.0, b.1 - a.1), (cc.0 - a.0, cc.1 - a.1));
        let gx = ((b.2 - a.2) * ac.1 - (cc.2 - a.2) * ab.1) / area;
        let gy = ((cc.2 - a.2) * ab.0 - (b.2 - a.2) * ac.0) / area;
        let ours = 1.0 + gx.hypot(gy) / 16.0;
        if (z - depth).abs() < ours + slack {
            return (None, false);
        }
        let pass = match s.func {
            gl::LESS | gl::LEQUAL => z < depth,
            gl::GREATER | gl::GEQUAL => z > depth,
            _ => true,
        };
        if pass {
            // The colour as the library rounds it from the 16.16 it was
            // given: the colour is not what is under test here.
            colour = colour_word(&[fx(c[0]), fx(c[1]), fx(c[2]), ONE]);
            if s.mask {
                (depth, slack) = (z, ours);
            }
        } else {
            hidden = true;
        }
    }
    (Some(colour), hidden)
}

/// Every scene's picture is the reference's, wherever the reference can
/// say; and enough pixels are compared, and enough of them are hidden,
/// that the depth test is what is being checked.
#[test]
fn depth_is_floating_points() {
    let mut r = Rng(0x0dd_ba11);
    let (mut compared, mut hidden, mut skipped) = (0, 0, 0);
    let mut frame = vec![[0u32; WORDS]; 4096];
    for k in 0..40 {
        let s = scene(&mut r, 6);
        let n = library(&s, &mut frame);
        let got = render(&decode_list(&frame[..n]), W as usize, H as usize);
        for y in 0..H as usize {
            for x in 0..W as usize {
                let (want, lost) = reference(&s, x, y);
                let Some(want) = want else {
                    skipped += 1;
                    continue;
                };
                let at = got[y * W as usize + x];
                assert_eq!(
                    at, want,
                    "scene {k}, pixel ({x}, {y}): {at:08x}, not {want:08x}"
                );
                compared += 1;
                hidden += lost as u32;
            }
        }
    }
    let all = 40 * W * H;
    assert!(
        compared > all * 3 / 4 && hidden > all / 25,
        "{compared} compared, {hidden} hidden, {skipped} skipped of {all}"
    );
}

/// The frame binned into tiles draws what the frame does: a tile's
/// depths start at the farthest, and the clear writes the clear's depth
/// into every tile it reaches.
#[test]
fn the_tile_table_draws_the_frame() {
    let mut r = Rng(0x5eed_1273);
    let mut frame = vec![[0u32; WORDS]; 4096];
    let mut entries = vec![[0u32; WORDS]; 16384];
    let mut tiles = vec![[0u32; TILE_WORDS]; 256];
    for _ in 0..10 {
        let s = scene(&mut r, 8);
        let n = library(&s, &mut frame);
        let flat = render(&decode_list(&frame[..n]), W as usize, H as usize);
        let words = frame[..n].to_vec();
        let b = razboj_tile::bin(&words, W, H, &mut entries, &mut tiles)
            .expect("room");
        let binned =
            render(&decode_list(&entries[..b.entries]), W as usize, H as usize);
        assert_eq!(binned, flat);
    }
}

/// The clears: one of the depth to the farthest before anything tests
/// depth has nothing to do, so a frame without depth stays a flat list;
/// one of both after depth was tested writes the depth, as one entry
/// with its depth plane's slot.
#[test]
fn a_clear_writes_depth_only_when_it_has_to() {
    let mut frame = vec![[0u32; WORDS]; 64];
    let mut g = Gl::new(&mut frame, W, H);
    g.clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
    g.clear(gl::DEPTH_BUFFER_BIT);
    assert_eq!(g.frame().len(), 1, "the colour's clear alone");
    assert!(!g.tiled());
    g.clear_depth(ONE / 2);
    g.clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
    assert_eq!(g.frame().len(), 3, "a clear to a nearer depth writes it");
    assert!(g.tiled());
    let plane = g.frame()[2][0] >> 12;
    assert_eq!(plane, 32768, "half of 65535, rounded");
    g.depth_func(gl::ALWAYS + 1);
    assert_eq!(g.get_error(), gl::INVALID_ENUM);
    // GL_STENCIL_BUFFER_BIT: there is no stencil.
    g.clear(0x0400);
    assert_eq!(g.get_error(), gl::INVALID_VALUE);
}
