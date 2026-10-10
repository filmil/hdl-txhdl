// SPDX-License-Identifier: Apache-2.0
//! The turning icosahedron through the GL ES library (issue 995): the
//! frames `ico_list.rs` writes by hand, written instead with
//! `glFrustumx`, `glRotatex`, `glLightxv` and `glDrawElements` from
//! `//gles`.
//!
//! This file has no hardware in it, as `ico_list.rs` has none, so the
//! board's program takes either, and a test on the host draws both
//! through Razboj's model and holds the two pictures to each other
//! (`docs/gles.md` section 8).
//!
//! ## The same picture
//!
//! * **The solid.** Faceted: every face its own three corners, all
//!   carrying the face's normal, sixty vertices drawn by
//!   `glDrawElements` with `GL_TRIANGLES`. The faces and their normals
//!   are `ico_list`'s, worked out the same way.
//! * **The view.** `ico_list` puts a point at `D - z` from the eye and
//!   scales by `PROJ` pixels. That is `glTranslatex` by `-D` and a
//!   frustum whose near plane is one unit away and `W / PROJ` units
//!   wide. The turn is `glRotatex` about the x axis after the y axis,
//!   a full turn being 256 of `ico_list`'s steps.
//! * **The light.** `ico_list` lights a face by how directly it faces
//!   the eye, `l`, as the square root of `0.4 + 0.6 l`, which spaces the
//!   shades evenly to the eye. GL's sum has no square root, but one
//!   directional light at the eye has its half vector at the eye too, so
//!   the specular term is `l` to the shininess. With the scene's ambient
//!   at the colour's full `sqrt(0.4)`, no diffuse, and the rest of the
//!   colour as specular at a shininess of 0.8715, GL's sum stays within
//!   0.004 of the square root over every `l`, under one step of a
//!   channel.
//! * **Hidden faces.** `glCullFace`, the back faces, as `ico_list` drops
//!   a face wound the wrong way on the screen; or, with `depth`, the
//!   depth test instead (#1273), every face drawn and each pixel the
//!   nearest face's, which for a convex solid is the same picture. A
//!   list that tests depth is a tile table's to draw. Flat shading.
//!
//! ## Two frames in one framebuffer
//!
//! GL's window has its rows upwards and Razboj's downwards, and the
//! library turns them over against the screen's height. So the second
//! frame, rows 512 to 991 of the framebuffer, is a screen of 992 rows
//! with a viewport of the lower 480 of GL's: those are Razboj's rows
//! 512 to 991. The backdrop's rectangle at the head of the list is
//! `ico_list`'s own, since a GL clear is the whole screen and would
//! take the other frame and the logo with it.
//!
//! ## Textured
//!
//! With a texture's room given, each face is textured as well (#997): a
//! checker of 32 by 32 texels, uploaded into the room each frame, since
//! a frame's context is new each time, at the nearest texel and repeated,
//! twice across each face, the lighting's colour modulating it.
//! Razboj textures only in a tile, as it tests depth only in one.
//!
//! Built with `--cfg=mip`, as `ico_mip_hdmi` (#997 step 4), the checker
//! is filtered with `GL_LINEAR_MIPMAP_LINEAR` and `GL_LINEAR`, its levels
//! generated as it is uploaded, and a floor recedes below the solid with
//! the checker repeated along it, so that its far rows read the levels
//! below the base.

use crate::ico_list::{rect, Box, Solid, BACKDROP, BODY, FACES, H, W, WORDS};
use gles::fixed::{Fx, ONE};
use gles::{gl, Gl, Vertex};

/// `ico_list`'s fixed point, ten bits of fraction, and the shift to
/// GL's sixteen.
const UP: u32 = 16 - 10;

/// `ico_list`'s view: the eye `D` from the centre, and `PROJ` pixels to
/// a unit at a unit's distance.
const D: Fx = 16 * ONE;
const PROJ: i64 = 1200;

/// The colour at full light, `ico_list`'s `LIT`, and the parts of it
/// that are ambient and specular, with the shininess, in 16.16.
const LIT: u32 = 0xdd_99_55;
const AMBIENT: Fx = 41448;
const SPECULAR: Fx = 24087;
const SHININESS: Fx = 57114;

/// Vertices drawn: three a face.
pub const VERTS: usize = 3 * FACES;

/// The most a frame's list holds: the backdrop's rectangle, and every
/// face with its second slot when the depth test is on and its two
/// texture slots when it is textured. Under `mirror` (#1592's step 8),
/// the reflection's faces as well, and the mirror's two triangles twice.
/// Under `blend` (step 7), the faces drawn twice, back then front, and
/// the stripes' triangles.
#[cfg(not(teapot))]
pub const MOST: usize = 1
    + 4 * FACES
    + 4 * FLOOR_TRIS
    + if cfg!(mirror) { 2 * FACES + 8 } else { 0 }
    + if cfg!(blend) {
        2 * FACES + 4 * STRIPES
    } else {
        0
    };
/// The canonical teapot's (#1592): the backdrop's rectangle, and each
/// triangle with its second slot.
#[cfg(teapot)]
pub const MOST: usize = 1 + 2 * crate::teapot::TRIANGLES;

/// The floor's triangles under `mip` (#997): two, each with its second
/// slot and its two texture slots.
#[cfg(mip)]
pub const FLOOR_TRIS: usize = 2;
#[cfg(not(mip))]
pub const FLOOR_TRIS: usize = 0;

/// The texture's side, in texels, and the side of one of its squares.
pub const TEX_SIDE: u32 = 32;
const SQUARE: u32 = 8;

/// The words a texture's room takes: the library's table of
/// descriptors, and the texture with its levels, rounded up.
pub const TEX_ROOM: usize = 4096;

/// The checker, RGBA bytes: squares of white and of a dark teal.
const fn checker() -> [u8; (TEX_SIDE * TEX_SIDE * 4) as usize] {
    let mut p = [0u8; (TEX_SIDE * TEX_SIDE * 4) as usize];
    let mut k = 0;
    while k < (TEX_SIDE * TEX_SIDE) as usize {
        let (i, j) = (k as u32 % TEX_SIDE, k as u32 / TEX_SIDE);
        let light = (i / SQUARE + j / SQUARE) & 1 == 0;
        let c: [u8; 4] = if light {
            [0xff, 0xff, 0xff, 0xff]
        } else {
            [0x20, 0x70, 0x80, 0xff]
        };
        p[4 * k] = c[0];
        p[4 * k + 1] = c[1];
        p[4 * k + 2] = c[2];
        p[4 * k + 3] = c[3];
        k += 1;
    }
    p
}

/// The checker's texels.
static CHECKER: [u8; (TEX_SIDE * TEX_SIDE * 4) as usize] = checker();

/// The solid as GL takes it: each face's three corners, each with the
/// face's normal, and the indices that draw them.
pub struct Model {
    pub positions: [[Fx; 4]; VERTS],
    pub normals: [[Fx; 3]; VERTS],
    pub indices: [u16; VERTS],
    /// Each corner's texture coordinates: the texture twice across a
    /// face, so its edges repeat it.
    pub texcoords: [[Fx; 4]; VERTS],
    /// Each corner's colour under `blend` (#1592's step 7): the solid's
    /// ambient gold, its alpha from how high the corner is on the solid,
    /// whole at the bottom and nearly clear at the top, which is the part
    /// the stripes are behind.
    pub colours: [[Fx; 4]; VERTS],
}

impl Model {
    pub fn new(solid: &Solid) -> Model {
        let mut m = Model {
            positions: [[0; 4]; VERTS],
            normals: [[0; 3]; VERTS],
            indices: [0; VERTS],
            texcoords: [[0; 4]; VERTS],
            colours: [[0; 4]; VERTS],
        };
        let corners = [[0, 0], [2 * ONE, 0], [ONE, 2 * ONE]];
        let mut f = 0;
        while f < solid.found {
            let n = solid.normal[f];
            let mut c = 0;
            while c < 3 {
                let v = 3 * f + c;
                let p = BODY[solid.face[f][c]];
                m.positions[v] = [p[0] << UP, p[1] << UP, p[2] << UP, ONE];
                m.normals[v] = if cfg!(smooth) {
                    outward(p)
                } else {
                    [n[0] << UP, n[1] << UP, n[2] << UP]
                };
                m.indices[v] = v as u16;
                let [s, t] = corners[c];
                m.texcoords[v] = [s, t, 0, ONE];
                let down = ONE as i64 - outward(p)[1] as i64;
                let [r, g, b, _] = colour(AMBIENT);
                m.colours[v] = [r, g, b, ONE * 3 / 20 + (down * 17 / 40) as Fx];
                c += 1;
            }
            f += 1;
        }
        m
    }
}

/// Under `smooth` (#1592's step 3), a corner's normal: the way from the
/// centre to it, of unit length, the solid's surface there if it were the
/// sphere through its corners, so that smooth shading blends the light
/// across faces as on a ball.
fn outward(p: [i32; 3]) -> [Fx; 3] {
    let sq = p.iter().map(|&c| c as i64 * c as i64).sum::<i64>();
    let mut len = sq.max(1);
    let mut next = (len + sq / len) / 2;
    while next < len {
        len = next;
        next = (len + sq / len) / 2;
    }
    p.map(|c| (c as i64 * ONE as i64 / len.max(1)) as Fx)
}

/// A channel of `LIT` scaled by `k`, in 16.16.
fn part(k: Fx, shift: u32) -> Fx {
    ((((LIT >> shift) & 0xff) as i64 * k as i64) / 255) as Fx
}

/// The colour `LIT` scaled by `k`, with an alpha of one.
fn colour(k: Fx) -> [Fx; 4] {
    [part(k, 16), part(k, 8), part(k, 0), ONE]
}

/// `ico_list`'s angle, 256 to the turn, in GL's degrees.
fn degrees(a: i32) -> Fx {
    (a & 255) * (360 * ONE / 256)
}

/// One frame's list, as `ico_list::frame` writes it: into `out`, for the
/// frame `dy` rows down, the solid turned by `ay` and `ax`, the backdrop
/// first over `clear`, the back faces culled or, with `depth`, hidden by
/// the depth test, and with `tex`, a texture's room, the bus address
/// Razboj reads it at and whether to upload the texture into it, textured:
/// a room a frame filled before keeps its texture (#1433). Returns the slots written and the box
/// the faces fill now.
#[allow(clippy::too_many_arguments)] // The frame's own parameters.
pub fn frame<'a>(
    model: &Model,
    ay: i32,
    ax: i32,
    dy: i32,
    clear: Box,
    depth: bool,
    tex: Option<(&'a mut [u32], u32, bool)>,
    out: &'a mut [[u32; WORDS]; MOST],
) -> (usize, Box) {
    out[0] = rect(BACKDROP, clear, dy);
    let (_, rest) = out.split_at_mut(1);
    let mut g = Gl::new(rest, W as u32, (H + dy) as u32);
    g.viewport(0, 0, W, H);

    // A frustum one unit deep, `W / PROJ` wide and `H / PROJ` high.
    let half_w = ((W as i64 / 2) * ONE as i64 / PROJ) as Fx;
    let half_h = ((H as i64 / 2) * ONE as i64 / PROJ) as Fx;
    g.matrix_mode(gl::PROJECTION);
    g.load_identity();
    g.frustum(-half_w, half_w, -half_h, half_h, ONE, 32 * ONE);

    // The light at the eye, placed while the modelview is the identity.
    g.matrix_mode(gl::MODELVIEW);
    g.load_identity();
    g.light(gl::LIGHT0, gl::POSITION, &[0, 0, ONE, 0]);
    g.light(gl::LIGHT0, gl::AMBIENT, &[0, 0, 0, ONE]);
    g.light(gl::LIGHT0, gl::DIFFUSE, &[0, 0, 0, ONE]);
    g.light(gl::LIGHT0, gl::SPECULAR, &[ONE, ONE, ONE, ONE]);
    g.light_model(gl::LIGHT_MODEL_AMBIENT, &[ONE, ONE, ONE, ONE]);
    g.material(gl::FRONT_AND_BACK, gl::AMBIENT, &colour(AMBIENT));
    g.material(gl::FRONT_AND_BACK, gl::DIFFUSE, &[0, 0, 0, ONE]);
    g.material(gl::FRONT_AND_BACK, gl::SPECULAR, &colour(SPECULAR));
    g.material(gl::FRONT_AND_BACK, gl::SHININESS, &[SHININESS]);
    // The canonical teapot (#1592) is lit diffusely as well, from the
    // eye, three tenths of its colour ambient and seven diffuse, so that
    // its curves read as they turn away.
    #[cfg(teapot)]
    {
        g.light(gl::LIGHT0, gl::DIFFUSE, &[ONE, ONE, ONE, ONE]);
        g.material(gl::FRONT_AND_BACK, gl::AMBIENT, &colour(3 * ONE / 10));
        g.material(gl::FRONT_AND_BACK, gl::DIFFUSE, &colour(7 * ONE / 10));
    }
    g.enable(gl::LIGHTING);
    g.enable(gl::LIGHT0);
    if depth {
        g.enable(gl::DEPTH_TEST);
    } else {
        g.enable(gl::CULL_FACE);
    }
    g.shade_model(if cfg!(smooth) { gl::SMOOTH } else { gl::FLAT });
    let textured = tex.is_some();
    if let Some((room, bus, upload)) = tex {
        if upload {
            texture(&mut g, room, bus);
        } else {
            kept(&mut g, room, bus);
        }
    }

    #[cfg(fog)]
    fog(&mut g);

    #[cfg(mip)]
    if textured {
        floor(&mut g);
    }

    if cfg!(blend) {
        stripes(&mut g);
    }

    // `ico_list` turns about y and then about x.
    g.translate(0, 0, -distance(ax));
    #[cfg(mirror)]
    mirror(&mut g, model, ay, ax);
    // The canonical teapot (#1592) leans twenty degrees towards the eye
    // and turns about its own axis, once in 128 frames.
    #[cfg(not(teapot))]
    g.rotate(degrees(ax), ONE, 0, 0);
    #[cfg(teapot)]
    g.rotate(20 * ONE, ONE, 0, 0);
    g.rotate(degrees(ay), 0, ONE, 0);
    #[cfg(teapot)]
    {
        // The teapot in the solid's place, untextured.
        let _ = (model, textured);
        let t = crate::teapot::get();
        g.draw_elements(
            gl::TRIANGLES,
            &t.indices[..3 * t.triangles],
            &t.positions,
            None,
            Some(&t.normals),
        );
    }
    #[cfg(not(teapot))]
    if textured {
        g.draw_vertices(gl::TRIANGLES, VERTS, |k| Vertex {
            position: model.positions[k],
            colour: None,
            normal: Some(model.normals[k]),
            tex: Some(model.texcoords[k]),
        });
    } else if cfg!(blend) {
        translucent(&mut g, model);
    } else {
        g.draw_elements(
            gl::TRIANGLES,
            &model.indices,
            &model.positions,
            None,
            Some(&model.normals),
        );
    }

    // The box the faces fill: each triangle's box, as Razboj walks it,
    // back in the frame's own rows. A triangle that tests depth, bit 8
    // of its word 15, or is textured, bit 14, has its second slot after
    // it, and a textured one two texture slots after that.
    let drawn = g.frame();
    let mut b = Box {
        x0: W,
        y0: H,
        x1: -1,
        y1: -1,
    };
    let mut k = 0;
    while k < drawn.len() {
        let w = &drawn[k];
        b.x0 = b.x0.min((w[1] & 0xffff) as i32);
        b.y0 = b.y0.min((w[1] >> 16) as i32 - dy);
        b.x1 = b.x1.max((w[2] & 0xffff) as i32);
        b.y1 = b.y1.max((w[2] >> 16) as i32 - dy);
        let second = ((w[15] >> 8) | (w[15] >> 13) | (w[15] >> 14)) & 1;
        let texture = 2 * ((w[15] >> 14) & 1);
        k += 1 + (second + texture) as usize;
    }
    (1 + drawn.len(), b)
}

/// The checker uploaded into `room`, which Razboj reads at `bus`, and
/// bound as `g`'s texture: nearest, the lighting modulating it, on.
pub fn texture<'a>(g: &mut Gl<'a>, room: &'a mut [u32], bus: u32) {
    g.texture_room(room, bus);
    let mut name = [0u32];
    g.gen_textures(&mut name);
    g.bind_texture(gl::TEXTURE_2D, name[0]);
    let t2 = gl::TEXTURE_2D;
    #[cfg(not(mip))]
    {
        g.tex_parameter(t2, gl::TEXTURE_MIN_FILTER, gl::NEAREST);
        g.tex_parameter(t2, gl::TEXTURE_MAG_FILTER, gl::NEAREST);
    }
    // Under `mip`, the levels made from the base as it is uploaded, and
    // trilinear filtering (#997).
    #[cfg(mip)]
    {
        let lml = gl::LINEAR_MIPMAP_LINEAR;
        g.tex_parameter(t2, gl::GENERATE_MIPMAP, 1);
        g.tex_parameter(t2, gl::TEXTURE_MIN_FILTER, lml);
        g.tex_parameter(t2, gl::TEXTURE_MAG_FILTER, gl::LINEAR);
    }
    let (s, rgba, ub) = (TEX_SIDE, gl::RGBA, gl::UNSIGNED_BYTE);
    g.tex_image_2d(t2, 0, rgba, s, s, 0, rgba, ub, &CHECKER);
    let modulate = gl::MODULATE as Fx;
    g.tex_env(gl::TEXTURE_ENV, gl::TEXTURE_ENV_MODE, &[modulate]);
    g.enable(gl::TEXTURE_2D);
}

/// The checker as an earlier frame left it in `room` (#1433), bound as
/// `g`'s texture with nothing uploaded: the lighting modulating it, on.
pub fn kept<'a>(g: &mut Gl<'a>, room: &'a mut [u32], bus: u32) {
    g.texture_room_kept(room, bus);
    g.bind_texture(gl::TEXTURE_2D, 1);
    let modulate = gl::MODULATE as Fx;
    g.tex_env(gl::TEXTURE_ENV, gl::TEXTURE_ENV_MODE, &[modulate]);
    g.enable(gl::TEXTURE_2D);
}

/// Under `mirror` (#1592's step 8): the view tilted down about the
/// solid's centre, and a mirror below the solid with the solid's
/// reflection in it. The mirror's rectangle goes into the stencil, its
/// colour and depth unwritten; the solid is drawn turned over in the
/// mirror's plane where the stencil says the mirror is, so that nothing
/// of the reflection lies outside the mirror; then the mirror itself is
/// blended over it, a translucent blue-grey, its depth written. The
/// solid itself comes after, from the tilted view this leaves.
#[cfg(mirror)]
fn mirror(g: &mut Gl<'_>, model: &Model, ay: i32, ax: i32) {
    // Four units farther than `D`, so that the solid and its reflection
    // both fit; 25 degrees down; the solid lifted 1.2 units, so that it
    // sits above the screen's middle and its reflection below. The
    // mirror is 1.8 units below the solid's centre, past its reach, 4
    // units wide and from 2.4 units behind the centre to 4 before it: on
    // the screen, rows 214 to about 400 and short of column 488, where
    // the logo starts.
    let (y, half, back, front) =
        (-9 * ONE / 5, 2 * ONE, -12 * ONE / 5, 4 * ONE);
    g.translate(0, 0, -4 * ONE);
    g.rotate(25 * ONE, ONE, 0, 0);
    g.translate(0, 6 * ONE / 5, 0);
    let square = [
        [-half, y, back, ONE],
        [half, y, back, ONE],
        [half, y, front, ONE],
        [-half, y, front, ONE],
    ];
    let order = [0usize, 1, 2, 0, 2, 3];
    let draw_square = |g: &mut Gl<'_>| {
        g.draw_vertices(gl::TRIANGLES, order.len(), |k| Vertex {
            position: square[order[k]],
            colour: None,
            normal: Some([0, ONE, 0]),
            tex: None,
        })
    };
    g.enable(gl::STENCIL_TEST);
    g.stencil_func(gl::ALWAYS, 1, 0xff);
    g.stencil_op(gl::KEEP, gl::KEEP, gl::REPLACE);
    g.color_mask(false, false, false, false);
    g.depth_mask(false);
    draw_square(g);
    g.color_mask(true, true, true, true);
    g.depth_mask(true);

    g.stencil_func(gl::EQUAL, 1, 0xff);
    g.stencil_op(gl::KEEP, gl::KEEP, gl::KEEP);
    g.push_matrix();
    g.translate(0, 2 * y, 0);
    g.scale(ONE, -ONE, ONE);
    g.rotate(degrees(ax), ONE, 0, 0);
    g.rotate(degrees(ay), 0, ONE, 0);
    g.draw_elements(
        gl::TRIANGLES,
        &model.indices,
        &model.positions,
        None,
        Some(&model.normals),
    );
    g.pop_matrix();
    g.disable(gl::STENCIL_TEST);

    g.disable(gl::LIGHTING);
    g.enable(gl::BLEND);
    g.blend_func(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
    g.color(3 * ONE / 20, 9 * ONE / 50, ONE / 4, 9 * ONE / 20);
    draw_square(g);
    g.disable(gl::BLEND);
    g.enable(gl::LIGHTING);
}

/// The stripes behind the solid under `blend` (#1592's step 7): six
/// upright bars, teal and grey by turns, 24 units off, unlit and opaque,
/// so that the solid in front of them shows what blending does. On the
/// screen they run from row 60 to row 300, above the logo, and from
/// column 60 to 580.
pub const STRIPES: usize = 6;
fn stripes(g: &mut Gl<'_>) {
    let (z, top, bottom) = (-24 * ONE, 18 * ONE / 5, -6 * ONE / 5);
    let width = 52 * ONE / 30;
    let left = -26 * ONE / 5;
    g.disable(gl::LIGHTING);
    for k in 0..STRIPES as i32 {
        let x0 = left + k * width;
        let x1 = x0 + width;
        let c = if k % 2 == 0 {
            [ONE / 8, 7 * ONE / 16, ONE / 2, ONE]
        } else {
            [3 * ONE / 4, 3 * ONE / 4, 3 * ONE / 4, ONE]
        };
        let corners = [[x0, bottom], [x1, bottom], [x1, top], [x0, top]];
        let order = [0usize, 1, 2, 0, 2, 3];
        g.draw_vertices(gl::TRIANGLES, order.len(), |i| {
            let [x, y] = corners[order[i]];
            Vertex {
                position: [x, y, z, ONE],
                colour: Some(c),
                normal: None,
                tex: None,
            }
        });
    }
    g.enable(gl::LIGHTING);
}

/// The solid under `blend` (#1592's step 7), translucent: each corner's
/// colour from `colours`, which colour material makes the lit colour's
/// ambient and its alpha, so that smooth shading carries the alpha across
/// each face as a plane; blended over what is behind it, its back faces
/// first and its front ones after, without writing depth, so that each
/// face blends over the ones behind it.
fn translucent(g: &mut Gl<'_>, model: &Model) {
    g.enable(gl::BLEND);
    g.blend_func(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
    g.enable(gl::COLOR_MATERIAL);
    g.depth_mask(false);
    g.enable(gl::CULL_FACE);
    for cull in [gl::FRONT, gl::BACK] {
        g.cull_face(cull);
        g.draw_elements(
            gl::TRIANGLES,
            &model.indices,
            &model.positions,
            Some(&model.colours),
            Some(&model.normals),
        );
    }
    g.disable(gl::CULL_FACE);
    g.depth_mask(true);
    g.disable(gl::COLOR_MATERIAL);
    g.disable(gl::BLEND);
}

/// How far away the solid is at angle `ax`: `D`, or under `fog` (#1592's
/// step 6) from `D` out to `D + SWING` and back once a turn of `ax`, its
/// speed easing to nothing at either end, so that it recedes into the
/// fog and comes back out of it.
fn distance(ax: i32) -> Fx {
    if !cfg!(fog) {
        return D;
    }
    const SWING: i64 = 10 * ONE as i64;
    let a = (ax & 255) as i64;
    let t = (if a < 128 { a } else { 256 - a }) * ONE as i64 / 128;
    let eased = t * t / ONE as i64 * (3 * ONE as i64 - 2 * t) / ONE as i64;
    D + (eased * SWING / ONE as i64) as Fx
}

/// Under `fog` (#1592's step 6): linear fog from the floor's near edge
/// to its far one, in the backdrop's colour. The floor fades into the
/// backdrop towards the horizon, and the solid keeps four fifths of its
/// own colour at `D` and a quarter at its farthest.
#[cfg(fog)]
fn fog(g: &mut Gl<'_>) {
    g.enable(gl::FOG);
    g.fog(gl::FOG_MODE, gl::LINEAR as Fx);
    g.fog(gl::FOG_START, 12 * ONE);
    g.fog(gl::FOG_END, 31 * ONE);
    let b = |shift: u32| ((BACKDROP >> shift) & 0xff) as Fx * ONE / 255;
    g.fog_colour([b(16), b(8), b(0), ONE]);
}

/// Under `mip` (#997), a floor below the solid and behind it, receding
/// from 11.25 units to 31, a unit and a half down: on the screen, rows
/// 296 to 400 from the left edge to column 480. The logo's 144 square
/// pixels start at column 488 and row 328, and the frame's box, which the
/// next frame clears, must stay off them (`ico_mip_test` checks). The
/// texture repeats six times across it and sixteen times along it, so its far
/// edge minifies the checker several levels down. Drawn with the
/// modelview at the identity, before the solid's turn.
#[cfg(mip)]
fn floor(g: &mut Gl<'_>) {
    let (y, near, far) = (-3 * ONE / 2, -45 * ONE / 4, -31 * ONE);
    let corners = [
        ([-3 * ONE, y, near, ONE], [0, 0]),
        ([3 * ONE / 2, y, near, ONE], [6 * ONE, 0]),
        ([3 * ONE / 2, y, far, ONE], [6 * ONE, 16 * ONE]),
        ([-3 * ONE, y, far, ONE], [0, 16 * ONE]),
    ];
    let order = [0, 1, 2, 0, 2, 3];
    g.draw_vertices(gl::TRIANGLES, order.len(), |k| {
        let (position, [s, t]) = corners[order[k]];
        Vertex {
            position,
            colour: None,
            normal: Some([0, ONE, 0]),
            tex: Some([s, t, 0, ONE]),
        }
    });
}
