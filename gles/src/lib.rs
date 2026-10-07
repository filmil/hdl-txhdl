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
use razboj_tile::{bin, clip, Binned, Bounds, Refused, TILE_WORDS, WORDS};

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

/// A vertex as a draw call reads it: its position in object
/// coordinates with its w, and its own colour and normal, or `None` for
/// the current ones.
#[derive(Clone, Copy, Debug, Default)]
pub struct Vertex {
    pub position: [Fx; 4],
    pub colour: Option<[Fx; 4]>,
    pub normal: Option<[Fx; 3]>,
}

/// A GL context drawing into a frame of Razboj's instructions.
pub struct Gl<'a> {
    frame: &'a mut [[u32; WORDS]],
    used: usize,
    screen: (u32, u32),
    /// The window's first row in Razboj's framebuffer: zero, unless
    /// EGL has put the window lower, on the half of a double buffer not
    /// shown (issue 996). Nothing is drawn above it.
    window_top: u32,
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
    point_size: Fx,
    line_width: Fx,
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
    /// Depth (#1273): the test's switch; its comparison, Razboj's, from
    /// nought for `GL_NEVER`; whether a pixel that passes writes its
    /// depth; the clear's depth; and the depth range, both in 16.16
    /// from nought to one.
    depth_test: bool,
    depth_func: u32,
    depth_mask: bool,
    clear_depth: Fx,
    depth_range: (Fx, Fx),
    /// Whether the frame holds an entry that tests depth, which Razboj
    /// draws only from a tile table.
    deep: bool,
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
            window_top: 0,
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
            point_size: ONE,
            line_width: ONE,
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
            depth_test: false,
            depth_func: gl::LESS - gl::NEVER,
            depth_mask: true,
            clear_depth: ONE,
            depth_range: (0, ONE),
            deep: false,
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
            gl::DEPTH_TEST => self.depth_test = on,
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
            gl::DEPTH_TEST => self.depth_test,
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

    /// `glPointSizex`: a point's size in pixels, rounded to a whole one
    /// and kept between one and [`gl::MAX_SIZE`] when it is drawn. Not
    /// above nought is `GL_INVALID_VALUE`.
    pub fn point_size(&mut self, size: Fx) {
        if size <= 0 {
            return self.fail(gl::INVALID_VALUE);
        }
        self.point_size = size;
    }

    /// `glLineWidthx`: a line's width in pixels, as a point's size is.
    pub fn line_width(&mut self, width: Fx) {
        if width <= 0 {
            return self.fail(gl::INVALID_VALUE);
        }
        self.line_width = width;
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

    /// `glDepthFunc`: the comparison a pixel's depth makes with the
    /// depth already there, `GL_NEVER` to `GL_ALWAYS`.
    pub fn depth_func(&mut self, func: u32) {
        match func {
            gl::NEVER..=gl::ALWAYS => self.depth_func = func - gl::NEVER,
            _ => self.fail(gl::INVALID_ENUM),
        }
    }

    /// `glDepthMask`: whether a pixel that passes writes its depth.
    pub fn depth_mask(&mut self, flag: bool) {
        self.depth_mask = flag;
    }

    /// `glClearDepthx`: the depth a clear writes, kept between nought
    /// and one.
    pub fn clear_depth(&mut self, depth: Fx) {
        self.clear_depth = depth.clamp(0, ONE);
    }

    /// `glDepthRangex`: where the near and the far planes fall in the
    /// depth's range, each kept between nought and one.
    pub fn depth_range(&mut self, near: Fx, far: Fx) {
        self.depth_range = (near.clamp(0, ONE), far.clamp(0, ONE));
    }

    /// `glClear` of the colour buffer, the depth buffer, or both, at
    /// this point of the frame.
    ///
    /// A clear of both is the clear's rectangle writing its colour and,
    /// over any depth, its depth. A tile's depth starts at the farthest
    /// (#992), so a clear of the depth to the farthest before anything
    /// in the frame has tested depth has nothing to do. A clear of the
    /// depth alone otherwise writes the depth and leaves the colour,
    /// which Razboj does not do until it has #993's masks: that clear is
    /// left undone, and `docs/gles.md` says so.
    pub fn clear(&mut self, mask: u32) {
        let both = gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT;
        if mask & !both != 0 {
            return self.fail(gl::INVALID_VALUE);
        }
        let z = depth_units(self.clear_depth);
        let depth =
            mask & gl::DEPTH_BUFFER_BIT != 0 && (self.deep || z != emit::FAR);
        if mask & gl::COLOR_BUFFER_BIT == 0 {
            return;
        }
        let colour = colour_word(&self.clear_colour);
        // Razboj's clear is of its own screen, the rows from zero; a
        // window lower down is cleared as a rectangle of itself, so that
        // the half of a double buffer being shown is left alone. A clear
        // of depth is a rectangle too, since only a tile table tests
        // depth and a tile table makes every clear one.
        let mut w = if self.window_top == 0 && !depth {
            emit::clear(colour)
        } else {
            let (sw, sh) = self.screen;
            emit::rect(colour, (0, self.window_top, sw - 1, sh - 1))
        };
        if depth {
            emit::depth(&mut w, gl::ALWAYS - gl::NEVER, true);
            self.push_pair(w, emit::flat_depth(z));
        } else {
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

    /// An entry that tests depth and its depth plane's slot, which go
    /// into the frame together or not at all.
    fn push_pair(&mut self, w: [u32; WORDS], slot: [u32; WORDS]) {
        if self.frame.len() - self.used < 2 {
            return self.fail(gl::OUT_OF_MEMORY);
        }
        self.frame[self.used] = w;
        self.frame[self.used + 1] = slot;
        self.used += 2;
        self.deep = true;
    }

    /// An entry as drawing makes it: testing depth as the context says
    /// when the depth test is on and its plane's slot `slot` is there,
    /// and as it is otherwise.
    fn push_drawn(&mut self, mut w: [u32; WORDS], slot: Option<[u32; WORDS]>) {
        match slot {
            Some(p) if self.depth_test => {
                emit::depth(&mut w, self.depth_func, self.depth_mask);
                self.push_pair(w, p);
            }
            _ => self.push(w),
        }
    }

    /// Draws from here on into `frame`, empty, as a window of `width`
    /// by `height` pixels whose first row is Razboj's row `top`, keeping
    /// every other part of the context's state: what EGL does at a swap,
    /// moving the window to the half of a double buffer not shown
    /// (issue 996). GL's window is the same size, so the viewport and
    /// everything else a program set stand.
    pub fn retarget(
        &mut self,
        frame: &'a mut [[u32; WORDS]],
        width: u32,
        height: u32,
        top: u32,
    ) {
        self.frame = frame;
        self.used = 0;
        self.deep = false;
        self.screen = (width, top + height);
        self.window_top = top;
    }

    /// The frame so far, as the instructions an untiled Razboj reads,
    /// an entry that tests depth followed by its depth plane's slot.
    pub fn frame(&self) -> &[[u32; WORDS]] {
        &self.frame[..self.used]
    }

    /// Whether the frame tests depth anywhere, so that Razboj has to
    /// draw it from a tile table, [`Gl::flush`]'s: a flat list draws an
    /// entry that tests depth as if it did not (#992).
    pub fn tiled(&self) -> bool {
        self.deep
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
        self.deep = false;
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
        self.draw(mode, n, |k| Vertex {
            position: positions[k],
            colour: colours.map(|c| c[k]),
            normal: normals.map(|v| v[k]),
        });
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
        self.draw(mode, indices.len(), |k| {
            let i = indices[k] as usize;
            Vertex {
                position: positions[i],
                colour: colours.map(|c| c[i]),
                normal: normals.map(|v| v[i]),
            }
        });
    }

    /// A draw call whose vertices are read one at a time: the `k`th of
    /// `count` is `vertex(k)`, in the order the mode says. This is what
    /// a caller with arrays of its own layout draws through, the C
    /// entry points' client arrays among them (issue 1224).
    pub fn draw_vertices(
        &mut self,
        mode: u32,
        count: usize,
        vertex: impl Fn(usize) -> Vertex,
    ) {
        self.draw(mode, count, vertex);
    }

    /// Records `error` as `glGetError` will report it, if no error is
    /// waiting already: for an entry point the library does not
    /// implement, which still links and says so (issue 1224).
    pub fn record_error(&mut self, error: u32) {
        self.fail(error);
    }

    /// The points, lines or triangles of a draw call: each of `count`
    /// vertices `vertex(k)`, in the order the mode says, the provoking
    /// vertex last.
    fn draw(
        &mut self,
        mode: u32,
        count: usize,
        vertex: impl Fn(usize) -> Vertex,
    ) {
        let prims = match mode {
            gl::POINTS => count,
            gl::LINES => count / 2,
            gl::LINE_STRIP => count.saturating_sub(1),
            gl::LINE_LOOP if count >= 2 => count,
            gl::LINE_LOOP => 0,
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
        let vert = |k: usize| {
            let v = vertex(k);
            let eye = matrix::mul_vec(&mv, &v.position);
            let clip = matrix::mul_vec(&pj, &eye);
            let col = v.colour.unwrap_or(current);
            if !lighting {
                return Vert {
                    eye,
                    clip,
                    col,
                    back: col,
                };
            }
            let n = matrix::mul3(&nm, &v.normal.unwrap_or(normal));
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
        // A strip's or a loop's vertex ends one segment and starts the
        // next, so it is lit once and carried over.
        let mut last = None;
        for t in 0..prims {
            match mode {
                gl::POINTS => self.point(vert(t)),
                gl::LINES => self.line(vert(2 * t), vert(2 * t + 1)),
                gl::LINE_STRIP | gl::LINE_LOOP => {
                    let a = last.unwrap_or_else(|| vert(t));
                    let b = vert((t + 1) % count);
                    self.line(a, b);
                    last = Some(b);
                }
                _ => {
                    let (a, b, c) = match mode {
                        gl::TRIANGLES => (3 * t, 3 * t + 1, 3 * t + 2),
                        gl::TRIANGLE_STRIP if t % 2 == 1 => (t + 1, t, t + 2),
                        gl::TRIANGLE_STRIP => (t, t + 1, t + 2),
                        _ => (0, t + 1, t + 2),
                    };
                    self.triangle([vert(a), vert(b), vert(c)]);
                }
            }
            if self.error == gl::OUT_OF_MEMORY {
                return;
            }
        }
    }

    /// The part of the screen GL draws on, which every instruction is
    /// clipped to: the window's rows, from its top down.
    fn bounds(&self) -> Bounds {
        let (sw, sh) = self.screen;
        (0, self.window_top, sw - 1, sh - 1)
    }

    /// A clipped vertex in GL's window, in sixteenths of a pixel, its y
    /// growing upwards: the divide and the viewport. `None` for a vertex
    /// at or behind the eye, which clipping leaves only in a degenerate
    /// case.
    fn window(&self, v: &Vert) -> Option<(i64, i64)> {
        let [x, y, _, w] = v.clip.map(|c| c as i64);
        if w <= 0 {
            return None;
        }
        let (vx, vy, vw, vh) = self.viewport;
        let wx =
            16 * vx as i64 + 8 * vw as i64 + div_round(x * 8 * vw as i64, w);
        let wy =
            16 * vy as i64 + 8 * vh as i64 + div_round(y * 8 * vh as i64, w);
        Some((wx, wy))
    }

    /// A clipped vertex's window depth, of sixteen bits: its z over its
    /// w, from minus one to one, carried into the depth range, then
    /// times 65535 and rounded to the nearest. Called only where
    /// [`Gl::window`] found the vertex in front of the eye.
    fn window_z(&self, v: &Vert) -> u32 {
        let (z, w) = (v.clip[2] as i128, v.clip[3] as i128);
        let (n, f) = (self.depth_range.0 as i128, self.depth_range.1 as i128);
        // n + (f - n) (z / w + 1) / 2, in 16.16, is
        // (2 n w + (f - n) (z + w)) / (2 w).
        let num = (2 * n * w + (f - n) * (z + w)) * emit::FAR as i128;
        let den = 2 * w * ONE as i128;
        (2 * num + den)
            .div_euclid(2 * den)
            .clamp(0, emit::FAR as i128) as u32
    }

    /// A point, as GL draws one that is not antialiased: kept only when
    /// its vertex is inside the clip volume and the user plane, then the
    /// square of its size in whole pixels, centred on the pixel the
    /// vertex is in when the size is odd and on the pixel corner nearest
    /// it when even, as one of Razboj's rectangles.
    fn point(&mut self, v: Vert) {
        let [x, y, z, w] = v.clip.map(|c| c as i64);
        let outside = |c: i64| c < -w || c > w;
        if outside(x) || outside(y) || outside(z) {
            return;
        }
        if self.plane_on && self.distances(&v, 2) < 0 {
            return;
        }
        let Some((wx, wy)) = self.window(&v) else {
            return;
        };
        let s = size(self.point_size);
        let lo = |c: i64| {
            if s % 2 == 1 {
                c.div_euclid(16) - (s - 1) / 2
            } else {
                (c + 8).div_euclid(16) - s / 2
            }
        };
        // GL's rows count up from the bottom, Razboj's down from the top.
        let sh = self.screen.1 as i64;
        let (x0, gy0) = (lo(wx), lo(wy));
        let (x1, y0, y1) = (x0 + s - 1, sh - gy0 - s, sh - 1 - gy0);
        let c = |v: i64| v.clamp(-(1 << 20), 1 << 20) as i32;
        if let Some(b) = clip(c(x0), c(y0), c(x1), c(y1), self.bounds()) {
            let slot = emit::flat_depth(self.window_z(&v));
            self.push_drawn(emit::rect(colour_word(&v.col), b), Some(slot));
        }
    }

    /// A line segment from `a` to `b`, clipped against the planes a
    /// triangle is, then drawn as the parallelogram GL's wide lines
    /// describe: the segment moved half the width up and down when it is
    /// more across than down, or left and right when not, as two of
    /// Razboj's triangles. Each column of an x-major line, or row of a
    /// y-major one, between its ends then holds as many pixels as the
    /// line is wide. Shaded smooth from one end's colour to the other's,
    /// or flat in `b`'s, the provoking vertex.
    ///
    /// GL's own rule for a line one pixel wide, the diamond exit, differs
    /// from this at the ends and in which of two pixels a column takes on
    /// a tie; it is left for the conformance tests (#999) to ask for.
    fn line(&mut self, a: Vert, b: Vert) {
        let flat = colour_word(&b.col);
        let (mut a, mut b) = (a, b);
        let guard = (3..7)
            .any(|p| self.distances(&a, p) < 0 || self.distances(&b, p) < 0);
        for p in 0..7 {
            if (p == 2 && !self.plane_on) || (p >= 3 && !guard) {
                continue;
            }
            let (da, db) = (self.distances(&a, p), self.distances(&b, p));
            match (da >= 0, db >= 0) {
                (false, false) => return,
                (true, false) => b = between(&a, &b, da, db),
                (false, true) => a = between(&a, &b, da, db),
                _ => {}
            }
        }
        let (Some(p), Some(q)) = (self.window(&a), self.window(&b)) else {
            return;
        };
        // Razboj's y, turned over.
        let sh16 = 16 * self.screen.1 as i64;
        let (p, q) = ((p.0, sh16 - p.1), (q.0, sh16 - q.1));
        let half = 8 * size(self.line_width);
        let o = if (q.0 - p.0).abs() >= (q.1 - p.1).abs() {
            (0, half)
        } else {
            (half, 0)
        };
        let at = |v: (i64, i64), s: i64| {
            let r = |c: i64| c.clamp(VMIN as i64, VMAX as i64) as i32;
            (r(v.0 + s * o.0), r(v.1 + s * o.1))
        };
        let quad = [at(p, -1), at(q, -1), at(q, 1), at(p, 1)];
        let (cp, cq) = (colour_word(&a.col), colour_word(&b.col));
        // Each corner's depth is its end's, as its colour is.
        let (zp, zq) = (self.window_z(&a), self.window_z(&b));
        let zs = self.depth_test.then_some(());
        let screen = self.bounds();
        for (i, j, k, s, z) in [
            (0, 1, 2, [cp, cq, cq], [zp, zq, zq]),
            (0, 2, 3, [cp, cq, cp], [zp, zq, zp]),
        ] {
            let (q, z) = ((quad[i], quad[j], quad[k]), zs.map(|_| z));
            let w = if self.smooth {
                emit::triangle(s[0], q.0, q.1, q.2, Some(s), z, screen)
            } else {
                emit::triangle(flat, q.0, q.1, q.2, None, z, screen)
            };
            if let Some((w, slot)) = w {
                self.push_drawn(w, slot);
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
        let sh16 = 16 * self.screen.1 as i64;
        let mut win = [(0i64, 0i64); MAXV];
        for (k, v) in poly[..n].iter().enumerate() {
            let Some(at) = self.window(v) else {
                return;
            };
            win[k] = at;
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
        let screen = self.bounds();
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
        // Each vertex's window depth, when the depth test wants it.
        let mut zw = [0u32; MAXV];
        if self.depth_test {
            for (k, v) in poly[..n].iter().enumerate() {
                zw[k] = self.window_z(v);
            }
        }
        for k in 1..n - 1 {
            let (a, b, c) = (at(0), at(k), at(k + 1));
            let z = self.depth_test.then_some([zw[0], zw[k], zw[k + 1]]);
            let w = if self.smooth {
                let s = [face(&poly[0]), face(&poly[k]), face(&poly[k + 1])]
                    .map(|c| colour_word(&c));
                emit::triangle(s[0], a, b, c, Some(s), z, screen)
            } else {
                emit::triangle(flat, a, b, c, None, z, screen)
            };
            if let Some((w, slot)) = w {
                self.push_drawn(w, slot);
            }
        }
    }
}

/// A depth from nought to one in 16.16 as sixteen bits, times 65535 and
/// rounded to the nearest, as a vertex's window depth is.
fn depth_units(d: Fx) -> u32 {
    let n = d.clamp(0, ONE) as i64 * emit::FAR as i64;
    ((n + (1 << 15)) >> 16) as u32
}

/// A point size or a line width in whole pixels: rounded to the
/// nearest, and kept between one and [`gl::MAX_SIZE`].
fn size(s: Fx) -> i64 {
    ((s as i64 + (1 << 15)) >> 16).clamp(1, gl::MAX_SIZE as i64)
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
