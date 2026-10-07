// SPDX-License-Identifier: Apache-2.0
//! The GL library's C entry points (issue 1224): `extern "C"` functions
//! with the names and types Khronos's `GLES/gl.h` declares, over the
//! Rust library in `//gles`, so that a C program includes `<GLES/gl.h>`
//! and links this as it would any GL ES 1.1 library. `docs/gles.md`
//! section 9 says why the library is Rust inside with C at the edge.
//!
//! GL's calls act on a current context. Making one current is EGL's
//! work, issue 996; until it lands, [`gles_make_current`] makes a
//! context current over a frame of Razboj's instructions its caller
//! owns, and [`gles_frame_len`] says how many the frame holds. Neither
//! is a GL call, and a program is not meant to call them: EGL is.
//!
//! The client arrays are read in the types Common-Lite allows, a
//! vertex at a time, so nothing is allocated and no array is copied.
//! The entry points `gl.h` declares that the library does not implement
//! are written from `gl.h` itself, `stubs.c`, and set
//! `GL_INVALID_OPERATION` through [`gles_record_error`].
//!
//! Nothing here panics across the boundary: every pointer a call is
//! given is the caller's to have made valid, as GL says, and a call
//! with no context current does nothing.
#![cfg_attr(not(feature = "std"), no_std)]
// The entry points take the caller's pointers, as GL's do; their
// validity is the caller's, and each is read only where GL reads it.
#![allow(clippy::missing_safety_doc)]

use core::ffi::c_void;
use gles::fixed::{Fx, ONE};
use gles::{gl, Gl, Vertex};
use razboj_tile::{Binned, TILE_WORDS, WORDS};

/// The enumerants `gles::gl` does not name, which the client arrays and
/// the queries use.
const BYTE: u32 = 0x1400;
const UNSIGNED_BYTE: u32 = 0x1401;
const SHORT: u32 = 0x1402;
const UNSIGNED_SHORT: u32 = 0x1403;
const FIXED: u32 = 0x140C;
const VERTEX_ARRAY: u32 = 0x8074;
const NORMAL_ARRAY: u32 = 0x8075;
const COLOR_ARRAY: u32 = 0x8076;
const VENDOR: u32 = 0x1F00;
const RENDERER: u32 = 0x1F01;
const VERSION: u32 = 0x1F02;
const EXTENSIONS: u32 = 0x1F03;

/// A client array: how many components, of what type, how far apart,
/// and where.
#[derive(Clone, Copy)]
struct Array {
    size: usize,
    kind: u32,
    stride: usize,
    at: *const u8,
    on: bool,
}

impl Array {
    const OFF: Array = Array {
        size: 4,
        kind: FIXED,
        stride: 0,
        at: core::ptr::null(),
        on: false,
    };

    /// The bytes one component takes.
    fn width(kind: u32) -> usize {
        match kind {
            BYTE | UNSIGNED_BYTE => 1,
            SHORT | UNSIGNED_SHORT => 2,
            _ => 4,
        }
    }

    /// Component `c` of element `i`, as the type stores it, widened.
    unsafe fn raw(&self, i: usize, c: usize) -> i64 {
        let w = Self::width(self.kind);
        let step = if self.stride == 0 {
            self.size * w
        } else {
            self.stride
        };
        let p = self.at.add(i * step + c * w);
        match self.kind {
            BYTE => (p as *const i8).read_unaligned() as i64,
            UNSIGNED_BYTE => p.read_unaligned() as i64,
            SHORT => (p as *const i16).read_unaligned() as i64,
            UNSIGNED_SHORT => (p as *const u16).read_unaligned() as i64,
            _ => (p as *const i32).read_unaligned() as i64,
        }
    }
}

/// The client arrays a draw call reads.
#[derive(Clone, Copy)]
struct Arrays {
    vertex: Array,
    colour: Array,
    normal: Array,
}

/// The current context: the library's, and its client arrays.
struct Context {
    gl: Gl<'static>,
    arrays: Arrays,
}

/// The one context current, on the one core this runs on.
static mut CURRENT: Option<Context> = None;

/// The current context, or `None` when none is.
fn current() -> Option<&'static mut Context> {
    // SAFETY: one core and no threads call GL on this machine, as on
    // every GL ES implementation a context is current to one thread.
    unsafe { (*core::ptr::addr_of_mut!(CURRENT)).as_mut() }
}

/// Makes a context current, drawing into `capacity` instructions of
/// Razboj's at `frame` for a screen of `width` by `height` pixels, in
/// GL's initial state. Not a GL call: EGL's, until issue 996 lands.
#[no_mangle]
pub unsafe extern "C" fn gles_make_current(
    frame: *mut u32,
    capacity: usize,
    width: u32,
    height: u32,
) {
    let entries =
        core::slice::from_raw_parts_mut(frame as *mut [u32; WORDS], capacity);
    *core::ptr::addr_of_mut!(CURRENT) = Some(Context {
        gl: Gl::new(entries, width, height),
        arrays: Arrays {
            vertex: Array::OFF,
            colour: Array::OFF,
            normal: Array::OFF,
        },
    });
}

/// Moves the current context to `capacity` instructions at `frame`,
/// empty, as a window of `width` by `height` pixels whose first row is
/// Razboj's row `top`, keeping the rest of its state: what EGL does at a
/// swap, issue 996. Not a GL call.
#[no_mangle]
pub unsafe extern "C" fn gles_retarget(
    frame: *mut u32,
    capacity: usize,
    width: u32,
    height: u32,
    top: u32,
) {
    let entries =
        core::slice::from_raw_parts_mut(frame as *mut [u32; WORDS], capacity);
    if let Some(c) = current() {
        c.gl.retarget(entries, width, height, top);
    }
}

/// How many instructions the current context's frame holds so far.
#[no_mangle]
pub extern "C" fn gles_frame_len() -> usize {
    current().map_or(0, |c| c.gl.frame().len())
}

/// Records `error` for `glGetError`, as an entry point the library does
/// not implement does (`stubs.c`).
#[no_mangle]
pub extern "C" fn gles_record_error(error: u32) {
    if let Some(c) = current() {
        c.gl.record_error(error);
    }
}

/// `n` values from `p`, at most four, the rest zero.
unsafe fn vals(p: *const Fx, n: usize) -> [Fx; 4] {
    let mut v = [0; 4];
    for (i, x) in v.iter_mut().enumerate().take(n) {
        *x = p.add(i).read_unaligned();
    }
    v
}

/// Runs `f` on the current context's library, if one is current.
fn with(f: impl FnOnce(&mut Gl<'static>)) {
    if let Some(c) = current() {
        f(&mut c.gl);
    }
}

#[no_mangle]
pub extern "C" fn glGetError() -> u32 {
    current().map_or(gl::NO_ERROR, |c| c.gl.get_error())
}

#[no_mangle]
pub extern "C" fn glViewport(x: i32, y: i32, width: i32, height: i32) {
    with(|g| g.viewport(x, y, width, height));
}

#[no_mangle]
pub extern "C" fn glMatrixMode(mode: u32) {
    with(|g| g.matrix_mode(mode));
}

#[no_mangle]
pub extern "C" fn glLoadIdentity() {
    with(|g| g.load_identity());
}

#[no_mangle]
pub unsafe extern "C" fn glLoadMatrixx(m: *const Fx) {
    let m: [Fx; 16] = core::array::from_fn(|i| m.add(i).read_unaligned());
    with(|g| g.load_matrix(&m));
}

#[no_mangle]
pub unsafe extern "C" fn glMultMatrixx(m: *const Fx) {
    let m: [Fx; 16] = core::array::from_fn(|i| m.add(i).read_unaligned());
    with(|g| g.mult_matrix(&m));
}

#[no_mangle]
pub extern "C" fn glPushMatrix() {
    with(|g| g.push_matrix());
}

#[no_mangle]
pub extern "C" fn glPopMatrix() {
    with(|g| g.pop_matrix());
}

#[no_mangle]
pub extern "C" fn glTranslatex(x: Fx, y: Fx, z: Fx) {
    with(|g| g.translate(x, y, z));
}

#[no_mangle]
pub extern "C" fn glRotatex(angle: Fx, x: Fx, y: Fx, z: Fx) {
    with(|g| g.rotate(angle, x, y, z));
}

#[no_mangle]
pub extern "C" fn glScalex(x: Fx, y: Fx, z: Fx) {
    with(|g| g.scale(x, y, z));
}

#[no_mangle]
pub extern "C" fn glFrustumx(l: Fx, r: Fx, b: Fx, t: Fx, n: Fx, f: Fx) {
    with(|g| g.frustum(l, r, b, t, n, f));
}

#[no_mangle]
pub extern "C" fn glOrthox(l: Fx, r: Fx, b: Fx, t: Fx, n: Fx, f: Fx) {
    with(|g| g.ortho(l, r, b, t, n, f));
}

#[no_mangle]
pub unsafe extern "C" fn glClipPlanex(plane: u32, equation: *const Fx) {
    let eq = vals(equation, 4);
    with(|g| g.clip_plane(plane, &eq));
}

#[no_mangle]
pub extern "C" fn glEnable(cap: u32) {
    with(|g| g.enable(cap));
}

#[no_mangle]
pub extern "C" fn glDisable(cap: u32) {
    with(|g| g.disable(cap));
}

#[no_mangle]
pub extern "C" fn glIsEnabled(cap: u32) -> u8 {
    current().map_or(0, |c| match cap {
        VERTEX_ARRAY => c.arrays.vertex.on as u8,
        COLOR_ARRAY => c.arrays.colour.on as u8,
        NORMAL_ARRAY => c.arrays.normal.on as u8,
        _ => c.gl.is_enabled(cap) as u8,
    })
}

#[no_mangle]
pub extern "C" fn glFrontFace(mode: u32) {
    with(|g| g.front_face(mode));
}

#[no_mangle]
pub extern "C" fn glCullFace(mode: u32) {
    with(|g| g.cull_face(mode));
}

#[no_mangle]
pub extern "C" fn glShadeModel(mode: u32) {
    with(|g| g.shade_model(mode));
}

#[no_mangle]
pub extern "C" fn glPointSizex(size: Fx) {
    with(|g| g.point_size(size));
}

#[no_mangle]
pub extern "C" fn glLineWidthx(width: Fx) {
    with(|g| g.line_width(width));
}

#[no_mangle]
pub extern "C" fn glColor4x(r: Fx, g: Fx, b: Fx, a: Fx) {
    with(|gl| gl.color(r, g, b, a));
}

/// A byte of colour as 16.16, 255 being one.
fn unit(c: u8) -> Fx {
    ((c as i64 * ONE as i64 + 127) / 255) as Fx
}

#[no_mangle]
pub extern "C" fn glColor4ub(r: u8, g: u8, b: u8, a: u8) {
    with(|gl| gl.color(unit(r), unit(g), unit(b), unit(a)));
}

#[no_mangle]
pub extern "C" fn glNormal3x(x: Fx, y: Fx, z: Fx) {
    with(|g| g.normal(x, y, z));
}

/// How many values a light's parameter takes.
fn light_count(pname: u32) -> usize {
    match pname {
        gl::AMBIENT | gl::DIFFUSE | gl::SPECULAR | gl::POSITION => 4,
        gl::SPOT_DIRECTION => 3,
        _ => 1,
    }
}

#[no_mangle]
pub extern "C" fn glLightx(light: u32, pname: u32, param: Fx) {
    with(|g| g.light(light, pname, &[param]));
}

#[no_mangle]
pub unsafe extern "C" fn glLightxv(light: u32, pname: u32, params: *const Fx) {
    let n = light_count(pname);
    let v = vals(params, n);
    with(|g| g.light(light, pname, &v[..n]));
}

#[no_mangle]
pub extern "C" fn glLightModelx(pname: u32, param: Fx) {
    with(|g| g.light_model(pname, &[param]));
}

#[no_mangle]
pub unsafe extern "C" fn glLightModelxv(pname: u32, params: *const Fx) {
    let n = if pname == gl::LIGHT_MODEL_AMBIENT {
        4
    } else {
        1
    };
    let v = vals(params, n);
    with(|g| g.light_model(pname, &v[..n]));
}

#[no_mangle]
pub extern "C" fn glMaterialx(face: u32, pname: u32, param: Fx) {
    with(|g| g.material(face, pname, &[param]));
}

#[no_mangle]
pub unsafe extern "C" fn glMaterialxv(
    face: u32,
    pname: u32,
    params: *const Fx,
) {
    let n = if pname == gl::SHININESS { 1 } else { 4 };
    let v = vals(params, n);
    with(|g| g.material(face, pname, &v[..n]));
}

#[no_mangle]
pub extern "C" fn glClearColorx(r: Fx, g: Fx, b: Fx, a: Fx) {
    with(|gl| gl.clear_color(r, g, b, a));
}

#[no_mangle]
pub extern "C" fn glClear(mask: u32) {
    with(|g| g.clear(mask));
}

#[no_mangle]
pub extern "C" fn glDepthFunc(func: u32) {
    with(|g| g.depth_func(func));
}

#[no_mangle]
pub extern "C" fn glDepthMask(flag: u8) {
    with(|g| g.depth_mask(flag != 0));
}

#[no_mangle]
pub extern "C" fn glBlendFunc(sfactor: u32, dfactor: u32) {
    with(|g| g.blend_func(sfactor, dfactor));
}

#[no_mangle]
pub extern "C" fn glAlphaFuncx(func: u32, reference: Fx) {
    with(|g| g.alpha_func(func, reference));
}

#[no_mangle]
pub extern "C" fn glColorMask(red: u8, green: u8, blue: u8, alpha: u8) {
    with(|g| g.color_mask(red != 0, green != 0, blue != 0, alpha != 0));
}

#[no_mangle]
pub extern "C" fn glClearDepthx(depth: Fx) {
    with(|g| g.clear_depth(depth));
}

#[no_mangle]
pub extern "C" fn glDepthRangex(near: Fx, far: Fx) {
    with(|g| g.depth_range(near, far));
}

/// Whether the current context's frame tests depth, so that it has to
/// be drawn from a tile table (#1273). Not a GL call: EGL's.
pub fn gles_frame_tiled() -> bool {
    current().is_some_and(|c| c.gl.tiled())
}

/// The current context's frame binned into tiles, into `entries` and
/// `tiles`, and a new frame begun: `None`, with the frame kept, when
/// the room is too small or no context is current. Not a GL call: what
/// EGL's swap does with a frame that tests depth (#1273).
pub fn gles_flush(
    entries: &mut [[u32; WORDS]],
    tiles: &mut [[u32; TILE_WORDS]],
) -> Option<Binned> {
    current().and_then(|c| c.gl.flush(entries, tiles).ok())
}

/// `glFlush` and `glFinish` leave the frame where it is: handing it to
/// Razboj and waiting for it are EGL's, issue 996, which reads the frame
/// [`gles_make_current`] was given and [`gles_frame_len`]'s count.
#[no_mangle]
pub extern "C" fn glFlush() {}

#[no_mangle]
pub extern "C" fn glFinish() {}

/// The strings `glGetString` answers. The version says what the
/// library is and that it is not conformant, as `docs/gles.md` section 1
/// asks, after the prefix ES 1.1 requires.
#[no_mangle]
pub extern "C" fn glGetString(name: u32) -> *const u8 {
    let s: &'static [u8] = match name {
        VENDOR => b"TxHDL\0",
        RENDERER => b"Razboj\0",
        VERSION => {
            b"OpenGL ES-CL 1.1 TxHDL, Common-Lite without textures, not conformant\0"
        }
        EXTENSIONS => b"\0",
        _ => {
            gles_record_error(gl::INVALID_ENUM);
            return core::ptr::null();
        }
    };
    s.as_ptr()
}

/// Which client array a state names.
fn array(c: &mut Context, which: u32) -> Option<&mut Array> {
    match which {
        VERTEX_ARRAY => Some(&mut c.arrays.vertex),
        COLOR_ARRAY => Some(&mut c.arrays.colour),
        NORMAL_ARRAY => Some(&mut c.arrays.normal),
        _ => None,
    }
}

fn client_state(which: u32, on: bool) {
    if let Some(c) = current() {
        match array(c, which) {
            Some(a) => a.on = on,
            None => c.gl.record_error(gl::INVALID_ENUM),
        }
    }
}

#[no_mangle]
pub extern "C" fn glEnableClientState(array: u32) {
    client_state(array, true);
}

#[no_mangle]
pub extern "C" fn glDisableClientState(array: u32) {
    client_state(array, false);
}

/// Sets a client array, if its size, type and stride are ones GL ES
/// 1.1 allows for it.
fn pointer(
    which: u32,
    size: i32,
    kind: u32,
    stride: i32,
    at: *const c_void,
    sizes: &[i32],
    kinds: &[u32],
) {
    let Some(c) = current() else {
        return;
    };
    if !sizes.contains(&size) || stride < 0 {
        return c.gl.record_error(gl::INVALID_VALUE);
    }
    if !kinds.contains(&kind) {
        return c.gl.record_error(gl::INVALID_ENUM);
    }
    if let Some(a) = array(c, which) {
        (a.size, a.kind, a.stride, a.at) =
            (size as usize, kind, stride as usize, at as *const u8);
    }
}

#[no_mangle]
pub extern "C" fn glVertexPointer(
    size: i32,
    kind: u32,
    stride: i32,
    at: *const c_void,
) {
    pointer(
        VERTEX_ARRAY,
        size,
        kind,
        stride,
        at,
        &[2, 3, 4],
        &[BYTE, SHORT, FIXED],
    );
}

#[no_mangle]
pub extern "C" fn glColorPointer(
    size: i32,
    kind: u32,
    stride: i32,
    at: *const c_void,
) {
    pointer(
        COLOR_ARRAY,
        size,
        kind,
        stride,
        at,
        &[4],
        &[UNSIGNED_BYTE, FIXED],
    );
}

#[no_mangle]
pub extern "C" fn glNormalPointer(kind: u32, stride: i32, at: *const c_void) {
    pointer(
        NORMAL_ARRAY,
        3,
        kind,
        stride,
        at,
        &[3],
        &[BYTE, SHORT, FIXED],
    );
}

/// A signed normalized component in 16.16: GL ES 1.1's `(2c + 1) /
/// (2^b - 1)` for a byte or a short of a normal.
fn signed_unit(c: i64, bits: u32) -> Fx {
    let full = (1i64 << bits) - 1;
    (((2 * c + 1) * ONE as i64) / full) as Fx
}

/// Element `i` of the current client arrays, as the library draws it.
unsafe fn fetch(c: &Arrays, i: usize) -> Vertex {
    let v = &c.vertex;
    let mut position = [0, 0, 0, ONE];
    for (k, p) in position.iter_mut().enumerate().take(v.size) {
        let raw = v.raw(i, k);
        *p = if v.kind == FIXED {
            raw as Fx
        } else {
            (raw << 16) as Fx
        };
    }
    let colour = c.colour.on.then(|| {
        core::array::from_fn(|k| {
            let raw = c.colour.raw(i, k);
            if c.colour.kind == FIXED {
                raw as Fx
            } else {
                unit(raw as u8)
            }
        })
    });
    let normal = c.normal.on.then(|| {
        core::array::from_fn(|k| {
            let raw = c.normal.raw(i, k);
            match c.normal.kind {
                BYTE => signed_unit(raw, 8),
                SHORT => signed_unit(raw, 16),
                _ => raw as Fx,
            }
        })
    });
    Vertex {
        position,
        colour,
        normal,
        tex: None,
    }
}

#[no_mangle]
pub extern "C" fn glDrawArrays(mode: u32, first: i32, count: i32) {
    let Some(c) = current() else {
        return;
    };
    if first < 0 || count < 0 {
        return c.gl.record_error(gl::INVALID_VALUE);
    }
    if !c.arrays.vertex.on {
        return;
    }
    let (first, count) = (first as usize, count as usize);
    let arrays = c.arrays;
    // SAFETY: the arrays were given by the caller for this draw, as GL
    // has it, and cover `first + count` elements.
    c.gl.draw_vertices(mode, count, |k| unsafe { fetch(&arrays, first + k) });
}

#[no_mangle]
pub unsafe extern "C" fn glDrawElements(
    mode: u32,
    count: i32,
    kind: u32,
    indices: *const c_void,
) {
    let Some(c) = current() else {
        return;
    };
    if count < 0 {
        return c.gl.record_error(gl::INVALID_VALUE);
    }
    if kind != UNSIGNED_BYTE && kind != UNSIGNED_SHORT {
        return c.gl.record_error(gl::INVALID_ENUM);
    }
    if !c.arrays.vertex.on {
        return;
    }
    let index = Array {
        size: 1,
        kind,
        stride: 0,
        at: indices as *const u8,
        on: true,
    };
    let arrays = c.arrays;
    c.gl.draw_vertices(mode, count as usize, |k| {
        fetch(&arrays, index.raw(k, 0) as usize)
    });
}
