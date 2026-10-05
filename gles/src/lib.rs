// SPDX-License-Identifier: Apache-2.0
//! GL ES 1.1 Common-Lite for Razboj: the fixed-point pipeline from a
//! vertex to Razboj's display list (`docs/gles.md`, issue 1159, the
//! first step of #995).
//!
//! No standard library and no allocation, so that a program on Vreteno
//! links it; the frame's room is the caller's. The calls are methods of
//! [`Gl`] named for the GL entry points they will stand behind once the
//! C ABI is in front of them, with GL's enumerants from [`gl`]. This
//! step has no lighting: a vertex's colour is its own or the current
//! one.
//!
//! A triangle goes through the steps of `docs/gles.md` section 3:
//! modelview and projection; clipping against the near and far planes,
//! the user plane, and Razboj's guard band, the last only when a vertex
//! lies outside it; the divide and the viewport, straight to Razboj's
//! sixteenths of a pixel with the window's y turned over; culling, in
//! GL's window, before the turn; and shading, flat from the provoking
//! vertex, the last, or smooth. Each piece of the clipped polygon goes
//! into the frame as one of Razboj's instructions, and [`Gl::flush`]
//! bins the frame into tiles with `razboj_tile`.
//!
//! The divide is one 64-bit division a window coordinate, rounded to the
//! nearest sixteenth, rather than the note's reciprocal of w in 2.30,
//! which cannot hold 1/w for a w under a quarter.
#![cfg_attr(not(test), no_std)]

pub mod emit;
pub mod fixed;
pub mod gl;
pub mod light;
pub mod matrix;

use emit::{VMAX, VMIN};
use fixed::{div, div_round, Fx, ONE};
use light::{Light, Material};
use matrix::{Mat, IDENTITY};
use razboj_tile::{bin, Binned, Bounds, Refused, TILE_WORDS, WORDS};

/// The guard band's edges in sixteenths, a pixel inside Razboj's range,
/// so that a vertex clipped onto the band and rounded stays in range.
const GMIN: i64 = VMIN as i64 + 16;
const GMAX: i64 = VMAX as i64 - 16;

/// The most vertices a clipped triangle has: three, and one for each of
/// the seven planes it can be clipped against.
const MAXV: usize = 10;

/// A vertex on its way: eye and clip coordinates, and its colour, and
/// with two-sided lighting the colour its back takes.
#[derive(Clone, Copy, Default)]
struct Vert {
    eye: [Fx; 4],
    clip: [Fx; 4],
    col: [Fx; 4],
    back: [Fx; 4],
}

/// A colour in 16.16, each channel nominally nought to one, as the
/// word Razboj takes, `0xAARRGGBB`: clamped, then `round(c * 255)`.
pub fn colour_word(c: &[Fx; 4]) -> u32 {
    let byte =
        |v: Fx| (((v.clamp(0, ONE) as i64 * 255) + (1 << 15)) >> 16) as u32;
    (byte(c[3]) << 24) | (byte(c[0]) << 16) | (byte(c[1]) << 8) | byte(c[2])
}

/// A GL context drawing into a frame of Razboj's instructions.
pub struct Gl<'a> {
    frame: &'a mut [[u32; WORDS]],
    used: usize,
    screen: (u32, u32),
    mode: u32,
    mv: [Mat; gl::MAX_MODELVIEW_STACK_DEPTH],
    mv_top: usize,
    pj: [Mat; gl::MAX_PROJECTION_STACK_DEPTH],
    pj_top: usize,
    viewport: (i32, i32, i32, i32),
    plane: [Fx; 4],
    plane_on: bool,
    cull_on: bool,
    cull: u32,
    front: u32,
    smooth: bool,
    colour: [Fx; 4],
    clear_colour: [Fx; 4],
    normal: [Fx; 3],
    lighting: bool,
    lights: [Light; gl::MAX_LIGHTS],
    material: Material,
    scene_ambient: [Fx; 4],
    two_side: bool,
    normalize: bool,
    rescale: bool,
    colour_material: bool,
    error: u32,
}

impl<'a> Gl<'a> {
    /// A context for a screen of `sw` by `sh` pixels, its frame held in
    /// `frame`, in GL's initial state: identity matrices, the viewport
    /// the whole screen, smooth shading, no culling, white.
    pub fn new(frame: &'a mut [[u32; WORDS]], sw: u32, sh: u32) -> Self {
        Gl {
            frame,
            used: 0,
            screen: (sw, sh),
            mode: gl::MODELVIEW,
            mv: [IDENTITY; gl::MAX_MODELVIEW_STACK_DEPTH],
            mv_top: 0,
            pj: [IDENTITY; gl::MAX_PROJECTION_STACK_DEPTH],
            pj_top: 0,
            viewport: (0, 0, sw as i32, sh as i32),
            plane: [0; 4],
            plane_on: false,
            cull_on: false,
            cull: gl::BACK,
            front: gl::CCW,
            smooth: true,
            colour: [ONE; 4],
            clear_colour: [0; 4],
            normal: [0, 0, ONE],
            lighting: false,
            lights: core::array::from_fn(Light::new),
            material: Material::default(),
            scene_ambient: [13107, 13107, 13107, ONE],
            two_side: false,
            normalize: false,
            rescale: false,
            colour_material: false,
            error: gl::NO_ERROR,
        }
    }

    /// `glGetError`: the first error since the last call, then none.
    pub fn get_error(&mut self) -> u32 {
        core::mem::replace(&mut self.error, gl::NO_ERROR)
    }

    fn fail(&mut self, e: u32) {
        if self.error == gl::NO_ERROR {
            self.error = e;
        }
    }

    /// The matrix the matrix calls act on.
    fn top(&mut self) -> &mut Mat {
        if self.mode == gl::PROJECTION {
            &mut self.pj[self.pj_top]
        } else {
            &mut self.mv[self.mv_top]
        }
    }

    /// The modelview and projection matrices now.
    pub fn modelview(&self) -> Mat {
        self.mv[self.mv_top]
    }
    pub fn projection(&self) -> Mat {
        self.pj[self.pj_top]
    }

    pub fn matrix_mode(&mut self, mode: u32) {
        match mode {
            gl::MODELVIEW | gl::PROJECTION => self.mode = mode,
            _ => self.fail(gl::INVALID_ENUM),
        }
    }

    pub fn load_identity(&mut self) {
        *self.top() = IDENTITY;
    }

    pub fn load_matrix(&mut self, m: &Mat) {
        *self.top() = *m;
    }

    /// `glMultMatrixx`: the current matrix times `m`, on the right.
    pub fn mult_matrix(&mut self, m: &Mat) {
        let t = self.top();
        *t = matrix::mul_mat(t, m);
    }

    pub fn push_matrix(&mut self) {
        if self.mode == gl::PROJECTION {
            if self.pj_top + 1 == self.pj.len() {
                return self.fail(gl::STACK_OVERFLOW);
            }
            self.pj[self.pj_top + 1] = self.pj[self.pj_top];
            self.pj_top += 1;
        } else {
            if self.mv_top + 1 == self.mv.len() {
                return self.fail(gl::STACK_OVERFLOW);
            }
            self.mv[self.mv_top + 1] = self.mv[self.mv_top];
            self.mv_top += 1;
        }
    }

    pub fn pop_matrix(&mut self) {
        let top = if self.mode == gl::PROJECTION {
            &mut self.pj_top
        } else {
            &mut self.mv_top
        };
        if *top == 0 {
            return self.fail(gl::STACK_UNDERFLOW);
        }
        *top -= 1;
    }

    pub fn translate(&mut self, x: Fx, y: Fx, z: Fx) {
        self.mult_matrix(&matrix::translate(x, y, z));
    }

    pub fn rotate(&mut self, deg: Fx, x: Fx, y: Fx, z: Fx) {
        self.mult_matrix(&matrix::rotate(deg, x, y, z));
    }

    pub fn scale(&mut self, x: Fx, y: Fx, z: Fx) {
        self.mult_matrix(&matrix::scale(x, y, z));
    }

    pub fn frustum(&mut self, l: Fx, r: Fx, b: Fx, t: Fx, n: Fx, f: Fx) {
        match matrix::frustum(l, r, b, t, n, f) {
            Some(m) => self.mult_matrix(&m),
            None => self.fail(gl::INVALID_VALUE),
        }
    }

    pub fn ortho(&mut self, l: Fx, r: Fx, b: Fx, t: Fx, n: Fx, f: Fx) {
        match matrix::ortho(l, r, b, t, n, f) {
            Some(m) => self.mult_matrix(&m),
            None => self.fail(gl::INVALID_VALUE),
        }
    }

    /// `glViewport`, in GL's window, whose origin is the bottom left.
    pub fn viewport(&mut self, x: i32, y: i32, w: i32, h: i32) {
        if w < 0 || h < 0 {
            return self.fail(gl::INVALID_VALUE);
        }
        self.viewport = (x, y, w, h);
    }

    /// `glClipPlanex` for plane 0, the one plane: carried into eye
    /// space by the modelview now.
    pub fn clip_plane(&mut self, plane: u32, eq: &[Fx; 4]) {
        if plane != gl::CLIP_PLANE0 {
            return self.fail(gl::INVALID_ENUM);
        }
        match matrix::plane_to_eye(&self.modelview(), eq) {
            Some(p) => self.plane = p,
            None => self.fail(gl::INVALID_OPERATION),
        }
    }

    /// The user plane in eye coordinates.
    pub fn eye_plane(&self) -> [Fx; 4] {
        self.plane
    }

    pub fn enable(&mut self, cap: u32) {
        self.switch(cap, true)
    }

    pub fn disable(&mut self, cap: u32) {
        self.switch(cap, false)
    }

    fn switch(&mut self, cap: u32, on: bool) {
        match cap {
            gl::CULL_FACE => self.cull_on = on,
            gl::CLIP_PLANE0 => self.plane_on = on,
            gl::LIGHTING => self.lighting = on,
            gl::NORMALIZE => self.normalize = on,
            gl::RESCALE_NORMAL => self.rescale = on,
            gl::COLOR_MATERIAL => self.colour_material = on,
            l if (gl::LIGHT0..gl::LIGHT0 + gl::MAX_LIGHTS as u32)
                .contains(&l) =>
            {
                self.lights[(l - gl::LIGHT0) as usize].on = on
            }
            _ => self.fail(gl::INVALID_ENUM),
        }
    }

    pub fn is_enabled(&self, cap: u32) -> bool {
        match cap {
            gl::CULL_FACE => self.cull_on,
            gl::CLIP_PLANE0 => self.plane_on,
            gl::LIGHTING => self.lighting,
            gl::NORMALIZE => self.normalize,
            gl::RESCALE_NORMAL => self.rescale,
            gl::COLOR_MATERIAL => self.colour_material,
            l if (gl::LIGHT0..gl::LIGHT0 + gl::MAX_LIGHTS as u32)
                .contains(&l) =>
            {
                self.lights[(l - gl::LIGHT0) as usize].on
            }
            _ => false,
        }
    }

    pub fn front_face(&mut self, mode: u32) {
        match mode {
            gl::CW | gl::CCW => self.front = mode,
            _ => self.fail(gl::INVALID_ENUM),
        }
    }

    pub fn cull_face(&mut self, mode: u32) {
        match mode {
            gl::FRONT | gl::BACK | gl::FRONT_AND_BACK => self.cull = mode,
            _ => self.fail(gl::INVALID_ENUM),
        }
    }

    pub fn shade_model(&mut self, mode: u32) {
        match mode {
            gl::FLAT => self.smooth = false,
            gl::SMOOTH => self.smooth = true,
            _ => self.fail(gl::INVALID_ENUM),
        }
    }

    /// `glColor4x`: the colour a vertex without its own takes.
    pub fn color(&mut self, r: Fx, g: Fx, b: Fx, a: Fx) {
        self.colour = [r, g, b, a];
    }

    /// `glNormal3x`: the normal a vertex without its own takes.
    pub fn normal(&mut self, x: Fx, y: Fx, z: Fx) {
        self.normal = [x, y, z];
    }

    /// `glLightxv`, and `glLightx` with one value: light `light`'s
    /// `pname`. The position and the spot direction are carried into eye
    /// space by the modelview now, as the specification says.
    pub fn light(&mut self, light: u32, pname: u32, params: &[Fx]) {
        let i = light.wrapping_sub(gl::LIGHT0) as usize;
        if i >= gl::MAX_LIGHTS {
            return self.fail(gl::INVALID_ENUM);
        }
        let want = match pname {
            gl::AMBIENT | gl::DIFFUSE | gl::SPECULAR | gl::POSITION => 4,
            gl::SPOT_DIRECTION => 3,
            gl::SPOT_EXPONENT
            | gl::SPOT_CUTOFF
            | gl::CONSTANT_ATTENUATION
            | gl::LINEAR_ATTENUATION
            | gl::QUADRATIC_ATTENUATION => 1,
            _ => return self.fail(gl::INVALID_ENUM),
        };
        if params.len() < want {
            return self.fail(gl::INVALID_VALUE);
        }
        let four = |p: &[Fx]| [p[0], p[1], p[2], p[3]];
        let mv = self.modelview();
        let v = params[0];
        let mut l = self.lights[i];
        match pname {
            gl::AMBIENT => l.ambient = four(params),
            gl::DIFFUSE => l.diffuse = four(params),
            gl::SPECULAR => l.specular = four(params),
            gl::POSITION => l.position = matrix::mul_vec(&mv, &four(params)),
            gl::SPOT_DIRECTION => {
                let d =
                    matrix::mul_vec(&mv, &[params[0], params[1], params[2], 0]);
                l.spot_direction = [d[0], d[1], d[2]];
            }
            gl::SPOT_EXPONENT if (0..=128 * ONE).contains(&v) => {
                l.spot_exponent = v
            }
            gl::SPOT_CUTOFF
                if (0..=90 * ONE).contains(&v) || v == 180 * ONE =>
            {
                l.spot_cutoff = v
            }
            gl::CONSTANT_ATTENUATION if v >= 0 => l.attenuation[0] = v,
            gl::LINEAR_ATTENUATION if v >= 0 => l.attenuation[1] = v,
            gl::QUADRATIC_ATTENUATION if v >= 0 => l.attenuation[2] = v,
            _ => return self.fail(gl::INVALID_VALUE),
        }
        self.lights[i] = l;
    }

    /// `glLightModelxv`, and `glLightModelx` with one value.
    pub fn light_model(&mut self, pname: u32, params: &[Fx]) {
        match pname {
            gl::LIGHT_MODEL_AMBIENT if params.len() >= 4 => {
                self.scene_ambient =
                    [params[0], params[1], params[2], params[3]]
            }
            gl::LIGHT_MODEL_TWO_SIDE if !params.is_empty() => {
                self.two_side = params[0] != 0
            }
            gl::LIGHT_MODEL_AMBIENT | gl::LIGHT_MODEL_TWO_SIDE => {
                self.fail(gl::INVALID_VALUE)
            }
            _ => self.fail(gl::INVALID_ENUM),
        }
    }

    /// `glMaterialxv`, and `glMaterialx` with one value. GL ES has one
    /// material for both faces, so `face` is `GL_FRONT_AND_BACK`.
    pub fn material(&mut self, face: u32, pname: u32, params: &[Fx]) {
        if face != gl::FRONT_AND_BACK {
            return self.fail(gl::INVALID_ENUM);
        }
        let want = match pname {
            gl::AMBIENT
            | gl::DIFFUSE
            | gl::SPECULAR
            | gl::EMISSION
            | gl::AMBIENT_AND_DIFFUSE => 4,
            gl::SHININESS => 1,
            _ => return self.fail(gl::INVALID_ENUM),
        };
        if params.len() < want {
            return self.fail(gl::INVALID_VALUE);
        }
        let c = || {
            [
                params[0],
                params[1],
                params[2],
                *params.get(3).unwrap_or(&0),
            ]
        };
        let m = &mut self.material;
        match pname {
            gl::AMBIENT => m.ambient = c(),
            gl::DIFFUSE => m.diffuse = c(),
            gl::SPECULAR => m.specular = c(),
            gl::EMISSION => m.emission = c(),
            gl::AMBIENT_AND_DIFFUSE => {
                m.ambient = c();
                m.diffuse = c();
            }
            _ if (0..=128 * ONE).contains(&params[0]) => {
                m.shininess = params[0]
            }
            _ => self.fail(gl::INVALID_VALUE),
        }
    }

    pub fn clear_color(&mut self, r: Fx, g: Fx, b: Fx, a: Fx) {
        self.clear_colour = [r, g, b, a];
    }

    /// `glClear` of the colour buffer: a clear at this point of the
    /// frame. The other buffers wait for their issues.
    pub fn clear(&mut self, mask: u32) {
        if mask & !gl::COLOR_BUFFER_BIT != 0 {
            return self.fail(gl::INVALID_VALUE);
        }
        if mask & gl::COLOR_BUFFER_BIT != 0 {
            let w = emit::clear(colour_word(&self.clear_colour));
            self.push(w);
        }
    }

    fn push(&mut self, w: [u32; WORDS]) {
        if self.used == self.frame.len() {
            return self.fail(gl::OUT_OF_MEMORY);
        }
        self.frame[self.used] = w;
        self.used += 1;
    }

    /// The frame so far, as the instructions an untiled Razboj reads.
    pub fn frame(&self) -> &[[u32; WORDS]] {
        &self.frame[..self.used]
    }

    /// `glFlush`: the frame binned into tiles, into the room given, and
    /// a new frame begun. With too little room nothing is written and
    /// the frame stays, so it can be binned again with more.
    pub fn flush(
        &mut self,
        entries: &mut [[u32; WORDS]],
        tiles: &mut [[u32; TILE_WORDS]],
    ) -> Result<Binned, Refused> {
        let (sw, sh) = self.screen;
        let r = bin(&self.frame[..self.used], sw, sh, entries, tiles)?;
        self.used = 0;
        Ok(r)
    }

    /// `glDrawArrays`: `positions` in object coordinates with their w,
    /// each vertex's colour from `colours` and its normal from `normals`
    /// or, without them, the current colour and normal.
    pub fn draw_arrays(
        &mut self,
        mode: u32,
        positions: &[[Fx; 4]],
        colours: Option<&[[Fx; 4]]>,
        normals: Option<&[[Fx; 3]]>,
    ) {
        let n = positions.len();
        if colours.is_some_and(|c| c.len() < n)
            || normals.is_some_and(|v| v.len() < n)
        {
            return self.fail(gl::INVALID_VALUE);
        }
        self.draw(mode, n, |k| k, positions, colours, normals);
    }

    /// `glDrawElements`: the same, the vertices taken by `indices`.
    pub fn draw_elements(
        &mut self,
        mode: u32,
        indices: &[u16],
        positions: &[[Fx; 4]],
        colours: Option<&[[Fx; 4]]>,
        normals: Option<&[[Fx; 3]]>,
    ) {
        let n = positions.len();
        if indices.iter().any(|&i| i as usize >= n)
            || colours.is_some_and(|c| c.len() < n)
            || normals.is_some_and(|v| v.len() < n)
        {
            return self.fail(gl::INVALID_VALUE);
        }
        self.draw(
            mode,
            indices.len(),
            |k| indices[k] as usize,
            positions,
            colours,
            normals,
        );
    }

    /// The triangles of a draw call: each of `count` vertices `at(k)`,
    /// in the order the mode says, the provoking vertex last.
    fn draw(
        &mut self,
        mode: u32,
        count: usize,
        at: impl Fn(usize) -> usize,
        positions: &[[Fx; 4]],
        colours: Option<&[[Fx; 4]]>,
        normals: Option<&[[Fx; 3]]>,
    ) {
        let tris = match mode {
            gl::TRIANGLES => count / 3,
            gl::TRIANGLE_STRIP | gl::TRIANGLE_FAN => count.saturating_sub(2),
            _ => return self.fail(gl::INVALID_ENUM),
        };
        let (mv, pj, current) =
            (self.modelview(), self.projection(), self.colour);
        // The normal matrix and the rescale factor, once a draw.
        let nm = if self.lighting {
            matrix::normal_matrix(&mv).unwrap_or([0; 9])
        } else {
            [0; 9]
        };
        let rescale = if self.rescale && !self.normalize {
            let len = light::length(&[nm[2], nm[5], nm[8]]);
            if len == 0 {
                ONE
            } else {
                div(ONE, len)
            }
        } else {
            ONE
        };
        // The lighting state the vertices read, copied out of the context
        // so that the triangles can be written into it meanwhile.
        let (lighting, normal, normalize, material, colour_material) = (
            self.lighting,
            self.normal,
            self.normalize,
            self.material,
            self.colour_material,
        );
        let (lights, scene, two_side) =
            (self.lights, self.scene_ambient, self.two_side);
        let vert = |i: usize| {
            let eye = matrix::mul_vec(&mv, &positions[i]);
            let clip = matrix::mul_vec(&pj, &eye);
            let col = colours.map_or(current, |c| c[i]);
            if !lighting {
                return Vert {
                    eye,
                    clip,
                    col,
                    back: col,
                };
            }
            let n = matrix::mul3(&nm, &normals.map_or(normal, |v| v[i]));
            let n = if normalize {
                light::normalize(&n)
            } else {
                n.map(|c| fixed::mul(c, rescale))
            };
            let mut m = material;
            if colour_material {
                (m.ambient, m.diffuse) = (col, col);
            }
            let lit = |n: &[Fx; 3]| light::shade(n, &eye, &m, &lights, &scene);
            let front = lit(&n);
            let back = if two_side { lit(&n.map(|c| -c)) } else { front };
            Vert {
                eye,
                clip,
                col: front,
                back,
            }
        };
        for t in 0..tris {
            let (a, b, c) = match mode {
                gl::TRIANGLES => (3 * t, 3 * t + 1, 3 * t + 2),
                gl::TRIANGLE_STRIP if t % 2 == 1 => (t + 1, t, t + 2),
                gl::TRIANGLE_STRIP => (t, t + 1, t + 2),
                _ => (0, t + 1, t + 2),
            };
            let tri = [vert(at(a)), vert(at(b)), vert(at(c))];
            self.triangle(tri);
            if self.error == gl::OUT_OF_MEMORY {
                return;
            }
        }
    }

    /// The planes a triangle is clipped against, each as a distance
    /// from it that is not negative inside: near and far, the user
    /// plane when it is on, and the guard band's four edges when some
    /// vertex is outside them.
    fn distances(&self, v: &Vert, plane: usize) -> i128 {
        let [x, y, z, w] = v.clip.map(|c| c as i64);
        let (vx, vy, vw, vh) = self.viewport;
        let (vx16, vy16) = (16 * vx as i64, 16 * vy as i64);
        let (vw8, vh8) = (8 * vw as i64, 8 * vh as i64);
        let sh16 = 16 * self.screen.1 as i64;
        // Window x in sixteenths is vx16 + vw8 + x vw8 / w, and GL's
        // window y vy16 + vh8 + y vh8 / w, which Razboj's y turns over
        // as sh16 less it; each edge is that kept inside the band, times
        // w, which is positive inside the near plane.
        (match plane {
            0 => z + w,
            1 => w - z,
            2 => {
                return (0..4)
                    .map(|i| self.plane[i] as i128 * v.eye[i] as i128)
                    .sum()
            }
            3 => x * vw8 + w * (vx16 + vw8 - GMIN),
            4 => w * (GMAX - vx16 - vw8) - x * vw8,
            5 => y * vh8 + w * (vy16 + vh8 - (sh16 - GMAX)),
            _ => w * ((sh16 - GMIN) - vy16 - vh8) - y * vh8,
        }) as i128
    }

    /// One triangle through clipping, the window, culling and shading
    /// into the frame.
    fn triangle(&mut self, tri: [Vert; 3]) {
        let mut poly = [Vert::default(); MAXV];
        poly[..3].copy_from_slice(&tri);
        let mut n = 3;
        let guard =
            (3..7).any(|p| tri.iter().any(|v| self.distances(v, p) < 0));
        for p in 0..7 {
            if (p == 2 && !self.plane_on) || (p >= 3 && !guard) {
                continue;
            }
            let mut out = [Vert::default(); MAXV];
            let mut m = 0;
            for i in 0..n {
                let (a, b) = (poly[i], poly[(i + 1) % n]);
                let (da, db) = (self.distances(&a, p), self.distances(&b, p));
                if da >= 0 {
                    out[m] = a;
                    m += 1;
                }
                if (da >= 0) != (db >= 0) {
                    out[m] = between(&a, &b, da, db);
                    m += 1;
                }
            }
            poly = out;
            n = m;
            if n < 3 {
                return;
            }
        }
        // The window, in sixteenths: GL's y, and Razboj's.
        let (vx, vy, vw, vh) = self.viewport;
        let sh16 = 16 * self.screen.1 as i64;
        let mut win = [(0i64, 0i64); MAXV];
        for (k, v) in poly[..n].iter().enumerate() {
            let [x, y, _, w] = v.clip.map(|c| c as i64);
            if w <= 0 {
                return;
            }
            let wx = 16 * vx as i64
                + 8 * vw as i64
                + div_round(x * 8 * vw as i64, w);
            let wy = 16 * vy as i64
                + 8 * vh as i64
                + div_round(y * 8 * vh as i64, w);
            win[k] = (wx, wy);
        }
        // Which way it faces, in GL's window, where counter-clockwise
        // has a positive area.
        let area: i64 = (0..n)
            .map(|k| {
                let (a, b) = (win[k], win[(k + 1) % n]);
                a.0 * b.1 - b.0 * a.1
            })
            .sum();
        if area == 0 {
            return;
        }
        let front = (area > 0) == (self.front == gl::CCW);
        if self.cull_on
            && (self.cull == gl::FRONT_AND_BACK
                || (self.cull == gl::FRONT && front)
                || (self.cull == gl::BACK && !front))
        {
            return;
        }
        let at = |k: usize| {
            let r = |v: i64| v.clamp(VMIN as i64, VMAX as i64) as i32;
            (r(win[k].0), r(sh16 - win[k].1))
        };
        let (sw, sh) = self.screen;
        let screen: Bounds = (0, 0, sw - 1, sh - 1);
        // With two-sided lighting a back face shows the colours lit for
        // its back.
        let back_face = self.two_side && self.lighting && !front;
        let face = |v: &Vert| {
            if back_face {
                v.back
            } else {
                v.col
            }
        };
        let flat = colour_word(&face(&tri[2]));
        for k in 1..n - 1 {
            let (a, b, c) = (at(0), at(k), at(k + 1));
            let w = if self.smooth {
                let s = [face(&poly[0]), face(&poly[k]), face(&poly[k + 1])]
                    .map(|c| colour_word(&c));
                emit::triangle(s[0], a, b, c, Some(s), screen)
            } else {
                emit::triangle(flat, a, b, c, None, screen)
            };
            if let Some(w) = w {
                self.push(w);
            }
        }
    }
}

/// The point where the edge from `a` to `b` crosses a plane, from their
/// distances to it, which have different signs: each coordinate and
/// each colour channel interpolated in clip space, as the
/// specification says, rounded to the nearest.
fn between(a: &Vert, b: &Vert, da: i128, db: i128) -> Vert {
    let lerp = |x: Fx, y: Fx| {
        let n = (y as i128 - x as i128) * da;
        let d = da - db;
        let (n, d) = if d < 0 { (-n, -d) } else { (n, d) };
        let t = (2 * n + d).div_euclid(2 * d);
        (x as i128 + t).clamp(i32::MIN as i128, i32::MAX as i128) as Fx
    };
    let mix =
        |p: &[Fx; 4], q: &[Fx; 4]| core::array::from_fn(|i| lerp(p[i], q[i]));
    Vert {
        eye: mix(&a.eye, &b.eye),
        clip: mix(&a.clip, &b.clip),
        col: mix(&a.col, &b.col),
        back: mix(&a.back, &b.back),
    }
}
