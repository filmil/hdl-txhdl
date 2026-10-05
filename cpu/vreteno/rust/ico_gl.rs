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
//!   a face wound the wrong way on the screen. Flat shading.
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

use crate::ico_list::{
    rect, Box, Solid, BACKDROP, BODY, FACES, H, MOST, W, WORDS,
};
use gles::fixed::{Fx, ONE};
use gles::{gl, Gl};

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

/// The solid as GL takes it: each face's three corners, each with the
/// face's normal, and the indices that draw them.
pub struct Model {
    pub positions: [[Fx; 4]; VERTS],
    pub normals: [[Fx; 3]; VERTS],
    pub indices: [u16; VERTS],
}

impl Model {
    pub fn new(solid: &Solid) -> Model {
        let mut m = Model {
            positions: [[0; 4]; VERTS],
            normals: [[0; 3]; VERTS],
            indices: [0; VERTS],
        };
        let mut f = 0;
        while f < solid.found {
            let n = solid.normal[f];
            let mut c = 0;
            while c < 3 {
                let v = 3 * f + c;
                let p = BODY[solid.face[f][c]];
                m.positions[v] = [p[0] << UP, p[1] << UP, p[2] << UP, ONE];
                m.normals[v] = [n[0] << UP, n[1] << UP, n[2] << UP];
                m.indices[v] = v as u16;
                c += 1;
            }
            f += 1;
        }
        m
    }
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
/// first over `clear`. Returns the entries written and the box the
/// faces fill now.
pub fn frame(
    model: &Model,
    ay: i32,
    ax: i32,
    dy: i32,
    clear: Box,
    out: &mut [[u32; WORDS]; MOST],
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
    g.enable(gl::LIGHTING);
    g.enable(gl::LIGHT0);
    g.enable(gl::CULL_FACE);
    g.shade_model(gl::FLAT);

    // `ico_list` turns about y and then about x.
    g.translate(0, 0, -D);
    g.rotate(degrees(ax), ONE, 0, 0);
    g.rotate(degrees(ay), 0, ONE, 0);
    g.draw_elements(
        gl::TRIANGLES,
        &model.indices,
        &model.positions,
        None,
        Some(&model.normals),
    );

    // The box the faces fill: each triangle's box, as Razboj walks it,
    // back in the frame's own rows.
    let drawn = g.frame();
    let mut b = Box {
        x0: W,
        y0: H,
        x1: -1,
        y1: -1,
    };
    for w in drawn {
        b.x0 = b.x0.min((w[1] & 0xffff) as i32);
        b.y0 = b.y0.min((w[1] >> 16) as i32 - dy);
        b.x1 = b.x1.max((w[2] & 0xffff) as i32);
        b.y1 = b.y1.max((w[2] >> 16) as i32 - dy);
    }
    (1 + drawn.len(), b)
}
