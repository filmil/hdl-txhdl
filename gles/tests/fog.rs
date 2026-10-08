// SPDX-License-Identifier: Apache-2.0
//! Fog through the GL library (#998), against floating point, through
//! Razboj's model.
//!
//! GL ES 1.1's three modes at a range of eye distances, a ramp across the
//! window whose factor the library carries as a plane, a point, and what
//! fog leaves alone: a clear, a pixel's alpha, and the state's queries
//! and errors. The reference fogs in `f64`, with the eye distance as
//! `|z_e|`, as GL allows and the library does.
use gles::fixed::{Fx, ONE};
use gles::{colour_word, gl, Gl};
use razboj::dl::decode_list;
use razboj::model::render_over;
use razboj_tile::WORDS;

const W: u32 = 64;
const H: u32 = 48;

fn fx(v: f64) -> Fx {
    (v * 65536.0).round() as Fx
}

/// The window drawn by the model as a tile table draws it.
fn drawn(frame: &[[u32; WORDS]], n: usize) -> Vec<u32> {
    render_over(
        &decode_list(&frame[..n]),
        W as usize,
        H as usize,
        vec![0; (W * H) as usize],
    )
}

/// A context with an orthographic projection that sees from the eye to
/// ten units in front of it, so that a vertex at `z` is `-z` away.
fn context(frame: &mut [[u32; WORDS]]) -> Gl<'_> {
    let mut g = Gl::new(frame, W, H);
    g.matrix_mode(gl::PROJECTION);
    g.ortho(-ONE, ONE, -ONE, ONE, 0, 10 * ONE);
    g.matrix_mode(gl::MODELVIEW);
    g
}

/// GL's fog factor at the distance `c`, clamped.
fn factor(mode: u32, c: f64, density: f64, start: f64, end: f64) -> f64 {
    let f = match mode {
        gl::LINEAR => (end - c) / (end - start),
        gl::EXP => (-density * c).exp(),
        _ => (-(density * c).powi(2)).exp(),
    };
    f.clamp(0.0, 1.0)
}

/// Each channel of `got` within `tol` of `c f + fog (1 - f)`, in bytes,
/// red, green and blue, and its alpha `c`'s.
fn near(got: u32, c: u32, fog: u32, f: f64, tol: f64) -> bool {
    let ch = |p: u32, k: u32| ((p >> (8 * k)) & 0xff) as f64;
    (0..3).all(|k| {
        let want = ch(c, k) * f + ch(fog, k) * (1.0 - f);
        (ch(got, k) - want).abs() <= tol
    }) && got >> 24 == c >> 24
}

/// A triangle over the whole window at the distance `c` takes GL's fog
/// factor there, in each of the three modes, at distances from before
/// the linear fog starts to past where it ends; and the alpha is kept.
#[test]
fn fog_is_gls_at_each_distance() {
    let s = [ONE * 3 / 4, ONE / 4, ONE, ONE / 2];
    let fc = [ONE / 8, ONE, ONE / 2, ONE];
    let (sw, fw) = (colour_word(&s), colour_word(&fc));
    let (density, start, end) = (0.3, 1.0, 6.0);
    for mode in [gl::LINEAR, gl::EXP, gl::EXP2] {
        for c in [0.25, 1.0, 2.5, 4.0, 5.75, 7.0, 9.5] {
            let mut frame = vec![[0u32; WORDS]; 16];
            let mut g = context(&mut frame);
            g.enable(gl::FOG);
            g.fog(gl::FOG_MODE, mode as Fx);
            g.fog(gl::FOG_DENSITY, fx(density));
            g.fog(gl::FOG_START, fx(start));
            g.fog(gl::FOG_END, fx(end));
            g.fog_colour(fc);
            let z = fx(-c);
            let p = [
                [-2 * ONE, -2 * ONE, z, ONE],
                [4 * ONE, -2 * ONE, z, ONE],
                [-2 * ONE, 4 * ONE, z, ONE],
            ];
            g.draw_arrays(gl::TRIANGLES, &p, Some(&[s; 3]), None);
            assert_eq!(g.get_error(), gl::NO_ERROR);
            let n = g.frame().len();
            let fb = drawn(&frame, n);
            let f = factor(mode, c, density, start, end);
            assert!(
                fb.iter().all(|&p| near(p, sw, fw, f, 1.5)),
                "mode {mode:04x} at {c}: {:08x}, f {f}",
                fb[0]
            );
        }
    }
}

/// A rectangle across the window from one unit away at its left edge to
/// six at its right, under linear fog from one to six: the factor falls
/// from one to nought across it, a plane, and each pixel's is GL's at its
/// centre.
#[test]
fn linear_fog_across_a_ramp_is_a_plane() {
    let s = [ONE, ONE, ONE, ONE];
    let fc = [0, 0, ONE / 4, ONE];
    let (sw, fw) = (colour_word(&s), colour_word(&fc));
    let (near_z, far_z) = (-ONE, -6 * ONE);
    let p = [
        [-ONE, -ONE, near_z, ONE],
        [ONE, -ONE, far_z, ONE],
        [ONE, ONE, far_z, ONE],
        [-ONE, ONE, near_z, ONE],
    ];
    let mut frame = vec![[0u32; WORDS]; 16];
    let mut g = context(&mut frame);
    g.enable(gl::FOG);
    g.fog(gl::FOG_MODE, gl::LINEAR as Fx);
    g.fog(gl::FOG_START, ONE);
    g.fog(gl::FOG_END, 6 * ONE);
    g.fog_colour(fc);
    g.draw_elements(
        gl::TRIANGLES,
        &[0, 1, 2, 0, 2, 3],
        &p,
        Some(&[s; 4]),
        None,
    );
    assert_eq!(g.get_error(), gl::NO_ERROR);
    let n = g.frame().len();
    let fb = drawn(&frame, n);
    for y in 0..H {
        for x in 0..W {
            let t = (x as f64 + 0.5) / W as f64;
            let f = 1.0 - t;
            let got = fb[(y * W + x) as usize];
            assert!(near(got, sw, fw, f, 2.0), "{x},{y}: {got:08x}, f {f}");
        }
    }
}

/// A point is fogged as a triangle at its distance is; a clear is not
/// fogged; and the state reads back as GL says, its errors included.
#[test]
fn a_point_is_fogged_and_a_clear_is_not() {
    let s = [ONE / 2, ONE, 0, ONE / 4];
    let fc = [ONE, 0, ONE, ONE];
    let (sw, fw) = (colour_word(&s), colour_word(&fc));
    let clear = [ONE / 4, ONE / 2, ONE * 3 / 4, ONE];
    let mut frame = vec![[0u32; WORDS]; 16];
    let mut g = context(&mut frame);
    g.enable(gl::FOG);
    g.fog(gl::FOG_DENSITY, fx(0.5));
    g.fog_colour(fc);
    g.clear_color(clear[0], clear[1], clear[2], clear[3]);
    g.clear(gl::COLOR_BUFFER_BIT);
    g.point_size(4 * ONE);
    g.draw_arrays(gl::POINTS, &[[0, 0, -2 * ONE, ONE]], Some(&[s]), None);
    assert_eq!(g.get_error(), gl::NO_ERROR);
    let n = g.frame().len();
    let fb = drawn(&frame, n);
    let f = factor(gl::EXP, 2.0, 0.5, 0.0, 1.0);
    let centre = fb[(H / 2 * W + W / 2) as usize];
    assert!(near(centre, sw, fw, f, 1.5), "the point: {centre:08x}");
    assert_eq!(fb[0], colour_word(&clear), "the clear is not fogged");

    let mut frame = vec![[0u32; WORDS]; 4];
    let mut g = Gl::new(&mut frame, W, H);
    let mut v = [0i32; 4];
    g.get_integer(gl::FOG_MODE, &mut v);
    assert_eq!(v[0] as u32, gl::EXP, "GL_EXP at first");
    let mut x = [0 as Fx; 4];
    g.get_fixed(gl::FOG_DENSITY, &mut x);
    assert_eq!(x[0], ONE);
    g.get_fixed(gl::FOG_END, &mut x);
    assert_eq!(x[0], ONE);
    assert!(!g.is_enabled(gl::FOG));
    g.fog_colour([-ONE, ONE / 2, 2 * ONE, ONE]);
    g.get_fixed(gl::FOG_COLOR, &mut x);
    assert_eq!(x, [0, ONE / 2, ONE, ONE], "clamped");
    g.fog(gl::FOG_START, -3 * ONE);
    g.get_fixed(gl::FOG_START, &mut x);
    assert_eq!(x[0], -3 * ONE);
    g.enable(gl::FOG);
    assert!(g.is_enabled(gl::FOG));
    assert_eq!(g.get_error(), gl::NO_ERROR);
    g.fog(gl::FOG_MODE, gl::LINEAR as Fx + 1);
    assert_eq!(g.get_error(), gl::INVALID_ENUM);
    g.fog(gl::FOG_DENSITY, -1);
    assert_eq!(g.get_error(), gl::INVALID_VALUE);
    g.fog(gl::FOG_COLOR, 0);
    assert_eq!(g.get_error(), gl::INVALID_ENUM);
    g.get_integer(gl::FOG_MODE, &mut v);
    assert_eq!(v[0] as u32, gl::EXP, "a refused mode changes nothing");
}
