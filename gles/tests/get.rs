// SPDX-License-Identifier: Apache-2.0
//! The queries (#1484): `glGetIntegerv`, `glGetFixedv` and
//! `glGetBooleanv` through the library, each name the library keeps
//! giving back what was set, converted as GL ES 1.1's section 6.1.2
//! says; the limits it states; and an unknown name an error.
use gles::fixed::{Fx, ONE};
use gles::get::name;
use gles::{gl, Gl};
use razboj_tile::WORDS;

/// A context over a frame of its own, for one test.
fn context(frame: &mut [[u32; WORDS]]) -> Gl<'_> {
    Gl::new(frame, 160, 120)
}

fn ints(g: &mut Gl, pname: u32, n: usize) -> Vec<i32> {
    let mut v = vec![i32::MIN; n];
    g.get_integer(pname, &mut v);
    assert_eq!(g.get_error(), gl::NO_ERROR, "{pname:04x}");
    v
}

fn fixed(g: &mut Gl, pname: u32, n: usize) -> Vec<Fx> {
    let mut v = vec![i32::MIN; n];
    g.get_fixed(pname, &mut v);
    assert_eq!(g.get_error(), gl::NO_ERROR, "{pname:04x}");
    v
}

fn bools(g: &mut Gl, pname: u32, n: usize) -> Vec<bool> {
    let mut v = vec![false; n];
    g.get_boolean(pname, &mut v);
    assert_eq!(g.get_error(), gl::NO_ERROR, "{pname:04x}");
    v
}

/// What GL keeps at first, and what each setter leaves.
#[test]
fn the_state_reads_back_as_set() {
    let mut frame = vec![[0u32; WORDS]; 16];
    let mut g = context(&mut frame);
    // At first.
    assert_eq!(ints(&mut g, name::VIEWPORT, 4), [0, 0, 160, 120]);
    assert_eq!(ints(&mut g, name::MATRIX_MODE, 1), [gl::MODELVIEW as i32]);
    assert_eq!(ints(&mut g, name::DEPTH_FUNC, 1), [gl::LESS as i32]);
    assert_eq!(ints(&mut g, name::SHADE_MODEL, 1), [gl::SMOOTH as i32]);
    assert_eq!(ints(&mut g, name::CULL_FACE_MODE, 1), [gl::BACK as i32]);
    assert_eq!(ints(&mut g, name::FRONT_FACE, 1), [gl::CCW as i32]);
    assert_eq!(ints(&mut g, name::BLEND_SRC, 1), [gl::ONE as i32]);
    assert_eq!(ints(&mut g, name::BLEND_DST, 1), [gl::ZERO as i32]);
    assert_eq!(bools(&mut g, name::COLOR_WRITEMASK, 4), [true; 4]);
    assert_eq!(bools(&mut g, name::DEPTH_WRITEMASK, 1), [true]);
    assert_eq!(fixed(&mut g, name::CURRENT_COLOR, 4), [ONE; 4]);
    assert_eq!(fixed(&mut g, name::DEPTH_RANGE, 2), [0, ONE]);
    let id: Vec<Fx> =
        (0..16).map(|k| if k % 5 == 0 { ONE } else { 0 }).collect();
    assert_eq!(fixed(&mut g, name::MODELVIEW_MATRIX, 16), id);
    assert_eq!(ints(&mut g, name::MODELVIEW_STACK_DEPTH, 1), [1]);
    assert_eq!(bools(&mut g, gl::DEPTH_TEST, 1), [false]);
    // As set.
    g.viewport(4, 8, 64, 32);
    g.matrix_mode(gl::PROJECTION);
    g.push_matrix();
    g.translate(ONE, 2 * ONE, 3 * ONE);
    g.depth_func(gl::GEQUAL);
    g.shade_model(gl::FLAT);
    g.cull_face(gl::FRONT);
    g.front_face(gl::CW);
    g.blend_func(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
    g.color_mask(true, false, true, false);
    g.depth_mask(false);
    g.color(ONE / 2, ONE / 4, 0, ONE);
    g.clear_color(ONE / 8, 0, ONE, ONE / 2);
    g.depth_range(ONE / 4, 3 * ONE / 4);
    g.enable(gl::DEPTH_TEST);
    g.alpha_func(gl::GREATER, ONE / 2);
    g.line_width(3 * ONE);
    assert_eq!(ints(&mut g, name::VIEWPORT, 4), [4, 8, 64, 32]);
    assert_eq!(ints(&mut g, name::MATRIX_MODE, 1), [gl::PROJECTION as i32]);
    assert_eq!(ints(&mut g, name::PROJECTION_STACK_DEPTH, 1), [2]);
    let m = fixed(&mut g, name::PROJECTION_MATRIX, 16);
    assert_eq!(&m[12..15], [ONE, 2 * ONE, 3 * ONE], "the translation");
    assert_eq!(ints(&mut g, name::DEPTH_FUNC, 1), [gl::GEQUAL as i32]);
    assert_eq!(ints(&mut g, name::SHADE_MODEL, 1), [gl::FLAT as i32]);
    assert_eq!(ints(&mut g, name::CULL_FACE_MODE, 1), [gl::FRONT as i32]);
    assert_eq!(ints(&mut g, name::FRONT_FACE, 1), [gl::CW as i32]);
    assert_eq!(ints(&mut g, name::BLEND_SRC, 1), [gl::SRC_ALPHA as i32]);
    assert_eq!(
        ints(&mut g, name::BLEND_DST, 1),
        [gl::ONE_MINUS_SRC_ALPHA as i32]
    );
    assert_eq!(
        bools(&mut g, name::COLOR_WRITEMASK, 4),
        [true, false, true, false]
    );
    assert_eq!(bools(&mut g, name::DEPTH_WRITEMASK, 1), [false]);
    assert_eq!(
        fixed(&mut g, name::CURRENT_COLOR, 4),
        [ONE / 2, ONE / 4, 0, ONE]
    );
    assert_eq!(
        fixed(&mut g, name::COLOR_CLEAR_VALUE, 4),
        [ONE / 8, 0, ONE, ONE / 2]
    );
    assert_eq!(fixed(&mut g, name::DEPTH_RANGE, 2), [ONE / 4, 3 * ONE / 4]);
    assert_eq!(bools(&mut g, gl::DEPTH_TEST, 1), [true]);
    assert_eq!(ints(&mut g, name::ALPHA_TEST_FUNC, 1), [gl::GREATER as i32]);
    assert_eq!(fixed(&mut g, name::LINE_WIDTH, 1), [3 * ONE]);
    assert_eq!(ints(&mut g, name::LINE_WIDTH, 1), [3]);
}

/// The conversions of section 6.1.2: a colour asked for as integers
/// maps one to the largest and nought to nought, a fixed value is
/// rounded, an integer asked for as fixed is that many ones, and
/// anything not nought is true.
#[test]
fn the_three_calls_convert_as_gl_says() {
    let mut frame = vec![[0u32; WORDS]; 16];
    let mut g = context(&mut frame);
    g.color(ONE, 0, ONE / 2, ONE);
    let c = ints(&mut g, name::CURRENT_COLOR, 4);
    assert_eq!((c[0], c[1], c[3]), (i32::MAX, 0, i32::MAX));
    let half = c[2] as f64 / i32::MAX as f64;
    assert!((half - 0.5).abs() < 1e-6, "a half maps to {half}");
    g.line_width(5 * ONE / 2 + 1);
    assert_eq!(ints(&mut g, name::LINE_WIDTH, 1), [3], "rounded");
    assert_eq!(
        fixed(&mut g, name::MAX_TEXTURE_SIZE, 1),
        [gl::MAX_TEXTURE_SIZE as Fx * ONE]
    );
    assert_eq!(bools(&mut g, name::MAX_LIGHTS, 1), [true]);
    assert_eq!(bools(&mut g, name::STENCIL_BITS, 1), [false]);
    assert_eq!(fixed(&mut g, gl::LIGHTING, 1), [0]);
}

/// The limits this implementation states, and an unknown name.
#[test]
fn the_limits_are_the_libraries() {
    let mut frame = vec![[0u32; WORDS]; 16];
    let mut g = context(&mut frame);
    assert_eq!(ints(&mut g, name::MAX_LIGHTS, 1), [8]);
    assert_eq!(ints(&mut g, name::MAX_CLIP_PLANES, 1), [1]);
    assert_eq!(ints(&mut g, name::MAX_TEXTURE_SIZE, 1), [1024]);
    assert_eq!(ints(&mut g, name::MAX_MODELVIEW_STACK_DEPTH, 1), [16]);
    assert_eq!(ints(&mut g, name::MAX_PROJECTION_STACK_DEPTH, 1), [2]);
    assert_eq!(ints(&mut g, name::MAX_TEXTURE_STACK_DEPTH, 1), [2]);
    assert_eq!(ints(&mut g, name::MAX_TEXTURE_UNITS, 1), [1]);
    assert_eq!(ints(&mut g, name::SUBPIXEL_BITS, 1), [4]);
    assert_eq!(ints(&mut g, name::RED_BITS, 1), [8]);
    assert_eq!(ints(&mut g, name::DEPTH_BITS, 1), [16]);
    assert_eq!(ints(&mut g, name::ALIASED_POINT_SIZE_RANGE, 2), [1, 64]);
    let mut v = [7i32; 2];
    g.get_integer(0x1234, &mut v);
    assert_eq!(g.get_error(), gl::INVALID_ENUM);
    assert_eq!(v, [7, 7], "an unknown name writes nothing");
}

/// The compressed formats the library takes (#998): the ten paletted
/// ones, listed in GL's order.
#[test]
fn the_compressed_formats_are_the_paletted_ones() {
    let mut frame = vec![[0u32; WORDS]; 16];
    let mut g = context(&mut frame);
    assert_eq!(ints(&mut g, name::NUM_COMPRESSED_TEXTURE_FORMATS, 1), [10]);
    let want: Vec<i32> = (0..10)
        .map(|k| (gl::PALETTE4_RGB8_OES + k) as i32)
        .collect();
    assert_eq!(ints(&mut g, name::COMPRESSED_TEXTURE_FORMATS, 10), want);
}

/// Polygon offset's switch, factor and units read back (#998).
#[test]
fn polygon_offset_reads_back() {
    let mut frame = vec![[0u32; WORDS]; 16];
    let mut g = context(&mut frame);
    assert_eq!(bools(&mut g, gl::POLYGON_OFFSET_FILL, 1), [false]);
    g.enable(gl::POLYGON_OFFSET_FILL);
    g.polygon_offset(-ONE, 3 * ONE / 2);
    assert_eq!(bools(&mut g, gl::POLYGON_OFFSET_FILL, 1), [true]);
    assert_eq!(fixed(&mut g, gl::POLYGON_OFFSET_FACTOR, 1), [-ONE]);
    assert_eq!(fixed(&mut g, gl::POLYGON_OFFSET_UNITS, 1), [3 * ONE / 2]);
    assert_eq!(ints(&mut g, gl::POLYGON_OFFSET_UNITS, 1), [2], "rounded");
}
