// SPDX-License-Identifier: Apache-2.0
//! The stencil through the GL library (#998), against a reference that
//! keeps its own stencil, through Razboj's model.
//!
//! Each round clears the stencil, writes a second value into the left half
//! of the window, and draws a quad over the whole window under one of the
//! eight comparisons, with an operation for each outcome, a value mask, a
//! write mask, and in some rounds a nearer quad over the top half that
//! fails it the depth test. The picture is held to the reference's, pixel
//! by pixel; then the same frame with one more quad for each stencil value
//! the reference holds, drawn only where the stencil is that value in a
//! colour that says it, so that the picture shows the stencil itself.
use gles::fixed::{Fx, ONE};
use gles::{colour_word, gl, Gl};
use razboj::dl::decode_list;
use razboj::model::render_over;
use razboj_tile::WORDS;

const W: u32 = 64;
const H: u32 = 48;

/// The window drawn by the model as a tile table draws it.
fn drawn(g: &Gl) -> Vec<u32> {
    assert!(g.tiled(), "a tile table");
    render_over(
        &decode_list(g.frame()),
        W as usize,
        H as usize,
        vec![0; (W * H) as usize],
    )
}

/// A quad from `x0` to `x1` and `y0` to `y1` in GL's coordinates, which
/// the identity matrices make the window's, at `z`, in `colour`.
fn quad(
    g: &mut Gl,
    (x0, x1, y0, y1): (Fx, Fx, Fx, Fx),
    z: Fx,
    colour: [Fx; 4],
) {
    let p = [
        [x0, y0, z, ONE],
        [x1, y0, z, ONE],
        [x1, y1, z, ONE],
        [x0, y1, z, ONE],
    ];
    g.draw_elements(
        gl::TRIANGLES,
        &[0, 1, 2, 0, 2, 3],
        &p,
        Some(&[colour; 4]),
        None,
    );
}

const WHOLE: (Fx, Fx, Fx, Fx) = (-ONE, ONE, -ONE, ONE);

/// GL's comparison `func`, nought for `GL_NEVER`, of `a` with `b`.
fn passes(func: u32, a: u32, b: u32) -> bool {
    [false, a < b, a == b, a <= b, a > b, a != b, a >= b, true][func as usize]
}

/// GL's six operations, in `gl`'s order of `KEEP`, `ZERO`, `REPLACE`,
/// `INCR`, `DECR` and `INVERT`, on the stencil `s` with the reference `r`.
const OPS: [u32; 6] = [
    gl::KEEP,
    gl::ZERO,
    gl::REPLACE,
    gl::INCR,
    gl::DECR,
    gl::INVERT,
];
fn op(k: usize, s: u32, r: u32) -> u32 {
    [s, 0, r, (s + 1).min(255), s.saturating_sub(1), !s & 0xff][k]
}

/// The colour that says the stencil `v`: red `v`, green `255 - v`, and
/// half a unit of blue.
fn says(v: u32) -> [Fx; 4] {
    let c = |b: u32| ((b as i64 * ONE as i64 + 127) / 255) as Fx;
    [c(v), c(255 - v), ONE / 2, ONE]
}

/// The stencil's test and operations are GL's under every comparison,
/// each operation in each outcome, masks of all bits and of some, with
/// and without a depth test failing some pixels, after a clear of the
/// stencil and a write of half of it.
#[test]
fn the_stencil_is_gls() {
    let back = [0, 0, ONE / 4, ONE];
    let ink = [ONE, ONE / 2, 0, ONE];
    for k in 0..24usize {
        let func = (k % 8) as u32;
        let ops = [k % 6, (k / 2 + 1) % 6, (k / 3 + 2) % 6];
        let reference = [0x35, 0xfe, 0x00, 0x80, 0x36, 0xff][k % 6];
        let mask = [0xff, 0x0f, 0xf3][k % 3];
        let write = [0xff, 0x3c, 0xf0][(k / 8) % 3];
        let deep = k % 2 == 1;
        let (c0, r1) = ([0x35, 0][(k / 4) % 2], 0xfe);
        // The reference: the stencil and whether each pixel was drawn.
        let mut want_s = vec![0u32; (W * H) as usize];
        let mut want_c = vec![colour_word(&back); (W * H) as usize];
        for y in 0..H {
            for x in 0..W {
                let s0 = if x < W / 2 { r1 } else { c0 };
                let sok = passes(func, reference & mask, s0 & mask);
                let zok = !(deep && y < H / 2);
                let o = if !sok {
                    ops[0]
                } else if !zok {
                    ops[1]
                } else {
                    ops[2]
                };
                let at = (y * W + x) as usize;
                want_s[at] = (s0 & !write) | (op(o, s0, reference) & write);
                if sok && zok {
                    want_c[at] = colour_word(&ink);
                }
            }
        }
        let mut values = want_s.clone();
        values.sort();
        values.dedup();
        for reveal in [false, true] {
            let mut frame = vec![[0u32; WORDS]; 128];
            let mut g = Gl::new(&mut frame, W, H);
            g.clear_color(back[0], back[1], back[2], back[3]);
            g.clear_stencil(c0 as i32);
            g.clear(gl::COLOR_BUFFER_BIT | gl::STENCIL_BUFFER_BIT);
            // The left half's stencil, and no colour.
            g.enable(gl::STENCIL_TEST);
            g.stencil_func(gl::ALWAYS, r1 as i32, 0xff);
            g.stencil_op(gl::KEEP, gl::KEEP, gl::REPLACE);
            g.color_mask(false, false, false, false);
            quad(&mut g, (-ONE, 0, -ONE, ONE), 0, back);
            // The nearer quad over the top half, which writes only depth.
            if deep {
                g.disable(gl::STENCIL_TEST);
                g.enable(gl::DEPTH_TEST);
                g.depth_func(gl::LESS);
                quad(&mut g, (-ONE, ONE, 0, ONE), -ONE / 2, back);
                g.enable(gl::STENCIL_TEST);
            }
            g.color_mask(true, true, true, true);
            g.stencil_func(gl::NEVER + func, reference as i32, mask);
            g.stencil_op(OPS[ops[0]], OPS[ops[1]], OPS[ops[2]]);
            g.stencil_mask(write);
            quad(&mut g, WHOLE, 0, ink);
            if reveal {
                g.disable(gl::DEPTH_TEST);
                g.stencil_op(gl::KEEP, gl::KEEP, gl::KEEP);
                for &v in &values {
                    g.stencil_func(gl::EQUAL, v as i32, 0xff);
                    quad(&mut g, WHOLE, 0, says(v));
                }
            }
            assert_eq!(g.get_error(), gl::NO_ERROR);
            let fb = drawn(&g);
            for (at, &got) in fb.iter().enumerate() {
                let want = if reveal {
                    colour_word(&says(want_s[at]))
                } else {
                    want_c[at]
                };
                let (x, y) = (at as u32 % W, at as u32 / W);
                assert_eq!(
                    got, want,
                    "round {k} at {x},{y}, reveal {reveal}: {got:08x} not {want:08x}"
                );
            }
        }
    }
}

/// A clear of the stencil alone leaves the depth as it was, and a clear
/// of the depth alone leaves the stencil.
#[test]
fn a_stencil_clear_keeps_the_depth() {
    let ink = [ONE, ONE, ONE, ONE];
    let mut frame = vec![[0u32; WORDS]; 64];
    let mut g = Gl::new(&mut frame, W, H);
    g.enable(gl::DEPTH_TEST);
    g.clear_depth(ONE / 4);
    g.clear(gl::DEPTH_BUFFER_BIT);
    g.clear_stencil(9);
    g.clear(gl::STENCIL_BUFFER_BIT);
    // At a depth of a half, behind the clear's: drawn nowhere.
    quad(&mut g, WHOLE, 0, ink);
    g.clear_depth(ONE);
    g.clear(gl::DEPTH_BUFFER_BIT);
    // The stencil the first clear left, nine, is still there.
    g.enable(gl::STENCIL_TEST);
    g.stencil_func(gl::EQUAL, 9, 0xff);
    quad(&mut g, (-ONE, 0, -ONE, ONE), 0, ink);
    assert_eq!(g.get_error(), gl::NO_ERROR);
    let fb = drawn(&g);
    for (at, &p) in fb.iter().enumerate() {
        let left = (at as u32 % W) < W / 2;
        assert_eq!(p == colour_word(&ink), left, "at {at}: {p:08x}");
    }
}

/// The stencil's state reads back as GL says, from GL's initial values,
/// and its errors are GL's.
#[test]
fn the_stencils_state_is_gls() {
    let mut frame = vec![[0u32; WORDS]; 4];
    let mut g = Gl::new(&mut frame, W, H);
    let get = |g: &mut Gl, pname: u32| {
        let mut v = [0i32; 4];
        g.get_integer(pname, &mut v);
        v[0]
    };
    assert_eq!(get(&mut g, gl::STENCIL_FUNC) as u32, gl::ALWAYS);
    assert_eq!(get(&mut g, gl::STENCIL_REF), 0);
    assert_eq!(get(&mut g, gl::STENCIL_VALUE_MASK), -1);
    assert_eq!(get(&mut g, gl::STENCIL_WRITEMASK), -1);
    assert_eq!(get(&mut g, gl::STENCIL_FAIL) as u32, gl::KEEP);
    assert_eq!(get(&mut g, gl::STENCIL_PASS_DEPTH_PASS) as u32, gl::KEEP);
    assert_eq!(get(&mut g, gl::STENCIL_CLEAR_VALUE), 0);
    assert_eq!(get(&mut g, gles::get::name::STENCIL_BITS), 8);
    assert!(!g.is_enabled(gl::STENCIL_TEST));
    g.stencil_func(gl::GEQUAL, 300, 0x0f);
    g.stencil_op(gl::INCR, gl::INVERT, gl::DECR);
    g.stencil_mask(0x3c);
    g.clear_stencil(7);
    g.enable(gl::STENCIL_TEST);
    assert_eq!(get(&mut g, gl::STENCIL_FUNC) as u32, gl::GEQUAL);
    assert_eq!(get(&mut g, gl::STENCIL_REF), 300, "kept as given");
    assert_eq!(get(&mut g, gl::STENCIL_VALUE_MASK), 0x0f);
    assert_eq!(get(&mut g, gl::STENCIL_FAIL) as u32, gl::INCR);
    assert_eq!(get(&mut g, gl::STENCIL_PASS_DEPTH_FAIL) as u32, gl::INVERT);
    assert_eq!(get(&mut g, gl::STENCIL_PASS_DEPTH_PASS) as u32, gl::DECR);
    assert_eq!(get(&mut g, gl::STENCIL_WRITEMASK), 0x3c);
    assert_eq!(get(&mut g, gl::STENCIL_CLEAR_VALUE), 7);
    assert!(g.is_enabled(gl::STENCIL_TEST));
    assert_eq!(g.get_error(), gl::NO_ERROR);
    g.stencil_func(gl::ALWAYS + 1, 0, 0);
    assert_eq!(g.get_error(), gl::INVALID_ENUM);
    g.stencil_op(gl::KEEP, gl::KEEP, gl::ONE);
    assert_eq!(g.get_error(), gl::INVALID_ENUM);
    assert_eq!(get(&mut g, gl::STENCIL_PASS_DEPTH_PASS) as u32, gl::DECR);
    g.clear(0x8000);
    assert_eq!(g.get_error(), gl::INVALID_VALUE);
}
