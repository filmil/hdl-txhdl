// SPDX-License-Identifier: Apache-2.0
//! Points and lines (#994), drawn by the library and rendered through
//! Razboj's model, against what GL says of them rather than against
//! the library's own construction.
//!
//! A line more across than down holds, in every column between its
//! ends, exactly as many pixels as it is wide, each within half the
//! width of the line; one more down than across the same in every row.
//! A point is the square of its size GL places on the pixel the vertex
//! is in, or on the corner nearest it. The screen is 128 pixels square,
//! with a projection that makes object coordinates GL's window pixels.
use gles::fixed::{Fx, ONE};
use gles::gl;
use gles::Gl;
use razboj::dl::decode;
use razboj::model::render;

const S: u32 = 128;
/// The current colour, white and opaque, as Razboj writes it.
const WHITE: u32 = 0xffff_ffff;

/// A context over `frame` with GL's window in object coordinates.
fn context(frame: &mut [[u32; 16]]) -> Gl<'_> {
    let mut g = Gl::new(frame, S, S);
    g.matrix_mode(gl::PROJECTION);
    g.ortho(0, S as Fx * ONE, 0, S as Fx * ONE, -ONE, ONE);
    g.matrix_mode(gl::MODELVIEW);
    g
}

/// A point in sixteenths of a pixel, as an object coordinate.
fn at(x16: i64, y16: i64) -> [Fx; 4] {
    [(x16 << 12) as Fx, (y16 << 12) as Fx, 0, ONE]
}

/// What the frame draws, a pixel of Razboj's rows a word.
fn picture(g: &Gl) -> Vec<u32> {
    let ops: Vec<_> = g.frame().iter().map(|w| decode(w)).collect();
    render(&ops, S as usize, S as usize)
}

/// A line from `p` to `q`, in GL's window in sixteenths, `w` wide, drawn
/// alone in white over black.
fn line(p: (i64, i64), q: (i64, i64), w: i32) -> Vec<u32> {
    let mut frame = [[0u32; 16]; 8];
    let mut g = context(&mut frame);
    g.line_width(w * ONE);
    g.draw_arrays(gl::LINES, &[at(p.0, p.1), at(q.0, q.1)], None, None);
    assert_eq!(g.get_error(), gl::NO_ERROR);
    picture(&g)
}

/// The line's columns, or rows, as GL's wide lines have them: across
/// the major axis, `w` pixels between its ends and none past them, each
/// within half the width of the line. Coordinates are Razboj's, rows
/// down, in sixteenths.
fn holds_its_width(fb: &[u32], p: (i64, i64), q: (i64, i64), w: i64) {
    let x_major = (q.0 - p.0).abs() >= (q.1 - p.1).abs();
    // Major and minor coordinates: (along, across).
    let mm = |v: (i64, i64)| if x_major { v } else { (v.1, v.0) };
    let (pm, qm) = (mm(p), mm(q));
    let (lo, hi) = (pm.0.min(qm.0), pm.0.max(qm.0));
    let n = S as i64;
    for k in 0..n {
        let c = 16 * k + 8;
        let lit: Vec<i64> = (0..n)
            .filter(|&j| {
                let (x, y) = if x_major { (k, j) } else { (j, k) };
                fb[(y * n + x) as usize] == WHITE
            })
            .collect();
        let want = if lo <= c && c < hi { w } else { 0 };
        assert_eq!(lit.len() as i64, want, "{p:?} to {q:?}, w {w}: {k}");
        // Within half the width: |across - line(c)| <= 8 w, multiplied
        // through by the run along the major axis.
        let run = qm.0 - pm.0;
        for j in lit {
            let off = (16 * j + 8 - pm.1) * run - (qm.1 - pm.1) * (c - pm.0);
            assert!(off.abs() <= 8 * w * run.abs(), "{p:?} to {q:?}: {k},{j}");
        }
    }
}

#[test]
fn lines_of_every_slope_hold_their_width() {
    let centre = (64 * 16 + 5, 63 * 16 + 11);
    for deg in (0..360).step_by(3) {
        let t = (deg as f64).to_radians();
        let r = 50.0 * 16.0;
        let end = (
            centre.0 + (r * t.cos()).round() as i64,
            centre.1 + (r * t.sin()).round() as i64,
        );
        for w in [1, 2, 3, 5] {
            let fb = line(centre, end, w);
            // GL's window has y up; Razboj's rows run down.
            let flip = |v: (i64, i64)| (v.0, 16 * S as i64 - v.1);
            holds_its_width(&fb, flip(centre), flip(end), w as i64);
        }
    }
}

/// A line far past the guard band is clipped to it and then to the
/// screen, and still holds its width in every column it crosses.
#[test]
fn a_line_past_the_guard_band_is_clipped() {
    let p = (64 * 16 + 8, 60 * 16 + 3);
    let q = (p.0 + 400_000, p.1 + 8_000);
    let fb = line(p, q, 1);
    let n = S as usize;
    for x in 0..n {
        let lit = (0..n).filter(|&y| fb[y * n + x] == WHITE).count();
        assert_eq!(lit, (x >= 64) as usize, "column {x}");
    }
}

#[test]
fn points_are_the_square_gl_places() {
    let n = S as i64;
    for s in 1..=8i64 {
        for (ox, oy) in [(0, 0), (3, 11), (8, 8), (15, 4), (7, 9)] {
            let v = (40 * 16 + ox, 70 * 16 + oy);
            let mut frame = [[0u32; 16]; 4];
            let mut g = context(&mut frame);
            g.point_size(s as Fx * ONE);
            g.draw_arrays(gl::POINTS, &[at(v.0, v.1)], None, None);
            let fb = picture(&g);
            // The square's centre, in GL's window in sixteenths: a pixel
            // centre for an odd size, a pixel corner for an even one.
            let centre = |c: i64| {
                if s % 2 == 1 {
                    16 * c.div_euclid(16) + 8
                } else {
                    16 * (c + 8).div_euclid(16)
                }
            };
            let (cx, cy) = (centre(v.0), centre(v.1));
            for row in 0..n {
                for x in 0..n {
                    // Razboj's row is GL's n - 1 - row.
                    let gy = n - 1 - row;
                    let inside = (2 * (16 * x + 8) - 2 * cx).abs() < 16 * s
                        && (2 * (16 * gy + 8) - 2 * cy).abs() < 16 * s;
                    let lit = fb[(row * n + x) as usize] == WHITE;
                    assert_eq!(lit, inside, "size {s} at {v:?}: {x},{row}");
                }
            }
        }
    }
}

/// A line's colours: smooth from one end's to the other's, or flat in
/// its second vertex's, the provoking one.
#[test]
fn a_line_is_shaded_from_end_to_end_or_flat() {
    let red = [ONE, 0, 0, 0];
    let blue = [0, 0, ONE, 0];
    // On the centres of GL's row 64, which is Razboj's 63.
    let ends = [at(10 * 16 + 8, 64 * 16 + 8), at(110 * 16 + 8, 64 * 16 + 8)];
    for smooth in [true, false] {
        let mut frame = [[0u32; 16]; 4];
        let mut g = context(&mut frame);
        g.shade_model(if smooth { gl::SMOOTH } else { gl::FLAT });
        g.draw_arrays(gl::LINES, &ends, Some(&[red, blue]), None);
        let fb = picture(&g);
        let row = (S - 1 - 64) as usize;
        let px = |x: usize| fb[row * S as usize + x];
        if smooth {
            assert!(px(10) >> 16 & 0xff > 0xf0 && px(10) & 0xff < 0x10);
            assert!(px(109) & 0xff > 0xf0 && px(109) >> 16 & 0xff < 0x10);
            let blues: Vec<u32> = (10..110).map(|x| px(x) & 0xff).collect();
            assert!(blues.windows(2).all(|w| w[0] <= w[1]), "{blues:?}");
        } else {
            assert!((10..110).all(|x| px(x) == 0x0000_00ff));
        }
    }
}

/// How many segments each mode makes of three vertices, each segment two
/// of Razboj's triangles; a point outside the clip volume is dropped;
/// and a size or width not above nought is refused.
#[test]
fn the_modes_make_their_segments_and_points() {
    let vs = [at(320, 320), at(1280, 400), at(700, 1500)];
    for (mode, entries) in [
        (gl::LINES, 2),
        (gl::LINE_STRIP, 4),
        (gl::LINE_LOOP, 6),
        (gl::POINTS, 3),
    ] {
        let mut frame = [[0u32; 16]; 8];
        let mut g = context(&mut frame);
        g.draw_arrays(mode, &vs, None, None);
        assert_eq!(g.frame().len(), entries, "mode {mode}");
    }
    let mut frame = [[0u32; 16]; 4];
    let mut g = context(&mut frame);
    g.draw_arrays(gl::POINTS, &[at(-16, 320), at(320, 16 * 200)], None, None);
    assert!(g.frame().is_empty(), "both outside the clip volume");
    g.point_size(0);
    assert_eq!(g.get_error(), gl::INVALID_VALUE);
    g.line_width(-ONE);
    assert_eq!(g.get_error(), gl::INVALID_VALUE);
}
