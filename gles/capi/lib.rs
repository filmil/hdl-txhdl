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
    /// The buffer object bound to `GL_ARRAY_BUFFER` when it was given,
    /// whose store `at` is an offset into; nought for none (#1488).
    buffer: u32,
}

impl Array {
    const OFF: Array = Array {
        size: 4,
        kind: FIXED,
        stride: 0,
        at: core::ptr::null(),
        on: false,
        buffer: 0,
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
    texcoord: Array,
}

/// The current context: the library's, and its client arrays.
struct Context {
    gl: Gl<'static>,
    arrays: Arrays,
    buffers: Buffers,
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
            texcoord: Array::OFF,
        },
        buffers: Buffers::NONE,
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

/// What draws the frame so far and gives the framebuffer back for
/// `glReadPixels` (#999): its words, each `0xAARRGGBB`, from Razboj's row
/// nought, and the words from one row to the next.
pub type Reader = fn() -> Option<(&'static [u32], usize)>;

/// The reader, which EGL sets when it makes a context current.
static mut READER: Option<Reader> = None;

/// Sets what `glReadPixels` reads through, or none, which leaves it
/// writing nothing. Not a GL call: EGL's (#999).
pub fn gles_reader(reader: Option<Reader>) {
    // SAFETY: one core and no threads, as for the current context.
    unsafe { *core::ptr::addr_of_mut!(READER) = reader };
}

/// `glReadPixels` (#999): the window's pixels from `x`, `y`, GL's, from
/// its bottom left, `width` by `height`, as `GL_RGBA` and
/// `GL_UNSIGNED_BYTE`, the rows from the bottom up, each padded to
/// `GL_PACK_ALIGNMENT`. The frame so far is drawn first. A pixel outside
/// the window is left as it was, which GL allows.
///
/// # Safety
///
/// `pixels` holds the rows the call writes, as GL says of it.
#[no_mangle]
pub unsafe extern "C" fn glReadPixels(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    format: u32,
    type_: u32,
    pixels: *mut u8,
) {
    let Some(c) = current() else {
        return;
    };
    let Some(((top, ww, wh), align)) =
        c.gl.read_pixels(width, height, format, type_)
    else {
        return;
    };
    let Some(read) = *core::ptr::addr_of!(READER) else {
        return;
    };
    let Some((fb, stride)) = read() else {
        return;
    };
    let row = (4 * width as usize).next_multiple_of(align);
    for j in 0..height {
        let gy = y as i64 + j as i64;
        if gy < 0 || gy >= wh as i64 {
            continue;
        }
        // GL's rows count up from the bottom, Razboj's down from the top.
        let r = top as usize + (wh as i64 - 1 - gy) as usize;
        for i in 0..width {
            let gx = x as i64 + i as i64;
            if gx < 0 || gx >= ww as i64 {
                continue;
            }
            let p = fb[r * stride + gx as usize];
            let at = pixels.add(j as usize * row + 4 * i as usize);
            let rgba =
                [(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8];
            core::ptr::copy_nonoverlapping(rgba.as_ptr(), at, 4);
        }
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
        gl::TEXTURE_COORD_ARRAY => c.arrays.texcoord.on as u8,
        _ => c.gl.is_enabled(cap) as u8,
    })
}

/// The client arrays' state, which the C API keeps rather than the
/// library (#1484): each array's switch, and its size, type and stride as
/// given, and the buffer objects' bindings (#1488), by GL ES 1.1's names
/// for them, each with how it converts (#1505): the types and the active
/// unit are enumerants, the rest integers; `None` for any other name.
fn array_state(
    a: &Arrays,
    b: &Buffers,
    pname: u32,
) -> Option<(i64, gles::get::Kind)> {
    use gles::get::Kind::{Enum, Integer};
    let size = |x: &Array| x.size as i64;
    let kind = |x: &Array| x.kind as i64;
    let stride = |x: &Array| x.stride as i64;
    let v = match pname {
        0x807B | 0x807E | 0x8082 | 0x8089 | 0x84E1 => {
            let t = match pname {
                0x807B => kind(&a.vertex),
                0x807E => kind(&a.normal),
                0x8082 => kind(&a.colour),
                0x8089 => kind(&a.texcoord),
                // GL_CLIENT_ACTIVE_TEXTURE: the one unit's.
                _ => 0x84C0,
            };
            return Some((t, Enum));
        }
        VERTEX_ARRAY => a.vertex.on as i64,
        NORMAL_ARRAY => a.normal.on as i64,
        COLOR_ARRAY => a.colour.on as i64,
        gl::TEXTURE_COORD_ARRAY => a.texcoord.on as i64,
        0x807A => size(&a.vertex),
        0x807C => stride(&a.vertex),
        0x807F => stride(&a.normal),
        0x8081 => size(&a.colour),
        0x8083 => stride(&a.colour),
        0x8088 => size(&a.texcoord),
        0x808A => stride(&a.texcoord),
        // The buffer bindings (#1488): the two targets', and the buffer
        // each array was given under.
        0x8894 => b.array as i64,
        0x8895 => b.element as i64,
        0x8896 => a.vertex.buffer as i64,
        0x8897 => a.normal.buffer as i64,
        0x8898 => a.colour.buffer as i64,
        0x889A => a.texcoord.buffer as i64,
        _ => return None,
    };
    Some((v, Integer))
}

/// A query (#1484): the client arrays' own state, or the library's.
/// `put` writes the values; an unknown name is `GL_INVALID_ENUM`.
unsafe fn query(pname: u32, put: impl FnOnce(&gles::get::Got)) {
    let Some(c) = current() else {
        return;
    };
    if let Some((v, kind)) = array_state(&c.arrays, &c.buffers, pname) {
        let mut values = [0i64; 16];
        values[0] = v;
        return put(&gles::get::Got {
            kind,
            values,
            len: 1,
        });
    }
    match c.gl.get(pname) {
        Some(g) => put(&g),
        None => c.gl.record_error(gl::INVALID_ENUM),
    }
}

#[no_mangle]
pub unsafe extern "C" fn glGetIntegerv(pname: u32, out: *mut i32) {
    if out.is_null() {
        return;
    }
    query(pname, |g| {
        g.integers(core::slice::from_raw_parts_mut(out, g.len))
    });
}

#[no_mangle]
pub unsafe extern "C" fn glGetFixedv(pname: u32, out: *mut Fx) {
    if out.is_null() {
        return;
    }
    query(pname, |g| {
        g.fixed(core::slice::from_raw_parts_mut(out, g.len))
    });
}

#[no_mangle]
pub unsafe extern "C" fn glGetBooleanv(pname: u32, out: *mut u8) {
    if out.is_null() {
        return;
    }
    query(pname, |g| {
        let mut b = [false; 16];
        g.booleans(&mut b[..g.len]);
        for (k, &v) in b[..g.len].iter().enumerate() {
            *out.add(k) = v as u8;
        }
    });
}

/// `glGetPointerv` (#1484): where each client array points, as given.
#[no_mangle]
pub unsafe extern "C" fn glGetPointerv(pname: u32, out: *mut *mut c_void) {
    let Some(c) = current() else {
        return;
    };
    if out.is_null() {
        return;
    }
    let a = &c.arrays;
    let at = match pname {
        0x808E => a.vertex.at,
        0x808F => a.normal.at,
        0x8090 => a.colour.at,
        0x8092 => a.texcoord.at,
        _ => return c.gl.record_error(gl::INVALID_ENUM),
    };
    *out = at as *mut c_void;
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

/// Writes `v`'s first `n` values to `out`, GL's array.
unsafe fn give<T: Copy>(out: *mut T, v: &[T], n: usize) {
    if out.is_null() {
        return;
    }
    for (k, &x) in v.iter().take(n).enumerate() {
        out.add(k).write_unaligned(x);
    }
}

/// `glGetLightxv` (#1511).
#[no_mangle]
pub unsafe extern "C" fn glGetLightxv(light: u32, pname: u32, params: *mut Fx) {
    let mut v = [0; 4];
    let mut n = 0;
    with(|g| n = g.get_light(light, pname, &mut v));
    give(params, &v, n);
}

/// `glGetMaterialxv` (#1511).
#[no_mangle]
pub unsafe extern "C" fn glGetMaterialxv(
    face: u32,
    pname: u32,
    params: *mut Fx,
) {
    let mut v = [0; 4];
    let mut n = 0;
    with(|g| n = g.get_material(face, pname, &mut v));
    give(params, &v, n);
}

/// `glGetTexParameteriv` (#1511): an enumerant, or a boolean as 1 or 0.
#[no_mangle]
pub unsafe extern "C" fn glGetTexParameteriv(
    target: u32,
    pname: u32,
    params: *mut i32,
) {
    let mut v = None;
    with(|g| v = g.get_tex_parameter(target, pname));
    if let Some(v) = v {
        give(params, &[v as i32], 1);
    }
}

/// `glGetTexParameterxv` (#1511): the same, an enumerant unscaled and a
/// boolean as 1.0 or 0.0, as section 6.1.2 converts them.
#[no_mangle]
pub unsafe extern "C" fn glGetTexParameterxv(
    target: u32,
    pname: u32,
    params: *mut Fx,
) {
    let mut v = None;
    with(|g| v = g.get_tex_parameter(target, pname));
    if let Some(v) = v {
        let x = if pname == gl::GENERATE_MIPMAP {
            v as Fx * ONE
        } else {
            v as Fx
        };
        give(params, &[x], 1);
    }
}

/// `glGetTexEnvxv` (#1511): the mode as an enumerant unscaled, the colour
/// in 16.16, and the sprites' replacement as 1.0 or 0.0.
#[no_mangle]
pub unsafe extern "C" fn glGetTexEnvxv(env: u32, pname: u32, params: *mut Fx) {
    let mut v = [0; 4];
    let mut n = 0;
    with(|g| n = g.get_tex_env(env, pname, &mut v));
    if pname == gl::COORD_REPLACE_OES {
        v[0] *= ONE;
    }
    give(params, &v, n);
}

/// `glGetTexEnviv` (#1511): the mode as an enumerant, the colour mapped
/// from [0, 1] onto the integers as Table 4.4 maps a colour, and the
/// sprites' replacement as 1 or 0.
#[no_mangle]
pub unsafe extern "C" fn glGetTexEnviv(env: u32, pname: u32, params: *mut i32) {
    let mut v = [0; 4];
    let mut n = 0;
    with(|g| n = g.get_tex_env(env, pname, &mut v));
    let w = if pname == gl::TEXTURE_ENV_COLOR {
        v.map(|c| ((c as i64 * i32::MAX as i64) / ONE as i64) as i32)
    } else {
        v
    };
    give(params, &w, n);
}

/// `glGetClipPlanex` (#1511): the plane's equation in eye coordinates.
#[no_mangle]
pub unsafe extern "C" fn glGetClipPlanex(plane: u32, eqn: *mut Fx) {
    let mut v = None;
    with(|g| v = g.get_clip_plane(plane));
    if let Some(v) = v {
        give(eqn, &v, 4);
    }
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
pub extern "C" fn glLogicOp(op: u32) {
    with(|g| g.logic_op(op));
}

#[no_mangle]
pub extern "C" fn glFogx(pname: u32, param: Fx) {
    with(|g| g.fog(pname, param));
}

/// `glFogxv` (#998): the fog's colour, four values, or one of the others,
/// one.
#[no_mangle]
pub unsafe extern "C" fn glFogxv(pname: u32, params: *const Fx) {
    if pname == gl::FOG_COLOR {
        let v = vals(params, 4);
        with(|g| g.fog_colour(v));
    } else {
        let v = vals(params, 1);
        with(|g| g.fog(pname, v[0]));
    }
}

#[no_mangle]
pub extern "C" fn glStencilFunc(func: u32, reference: i32, mask: u32) {
    with(|g| g.stencil_func(func, reference, mask));
}

#[no_mangle]
pub extern "C" fn glStencilOp(fail: u32, zfail: u32, zpass: u32) {
    with(|g| g.stencil_op(fail, zfail, zpass));
}

#[no_mangle]
pub extern "C" fn glStencilMask(mask: u32) {
    with(|g| g.stencil_mask(mask));
}

#[no_mangle]
pub extern "C" fn glClearStencil(s: i32) {
    with(|g| g.clear_stencil(s));
}

#[no_mangle]
pub extern "C" fn glScissor(x: i32, y: i32, w: i32, h: i32) {
    with(|g| g.scissor(x, y, w, h));
}

#[no_mangle]
pub extern "C" fn glHint(target: u32, mode: u32) {
    with(|g| g.hint(target, mode));
}

#[no_mangle]
pub extern "C" fn glDepthFunc(func: u32) {
    with(|g| g.depth_func(func));
}

#[no_mangle]
pub extern "C" fn glPolygonOffsetx(factor: Fx, units: Fx) {
    with(|g| g.polygon_offset(factor, units));
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
            b"OpenGL ES-CL 1.1 TxHDL, Common-Lite, one texture unit, not conformant\0"
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
        gl::TEXTURE_COORD_ARRAY => Some(&mut c.arrays.texcoord),
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
    let buffer = c.buffers.array;
    if let Some(a) = array(c, which) {
        (a.size, a.kind, a.stride, a.at) =
            (size as usize, kind, stride as usize, at as *const u8);
        a.buffer = buffer;
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
    let t = &c.texcoord;
    let tex = t.on.then(|| {
        let mut v = [0, 0, 0, ONE];
        for (k, p) in v.iter_mut().enumerate().take(t.size) {
            let raw = t.raw(i, k);
            *p = if t.kind == FIXED {
                raw as Fx
            } else {
                (raw << 16) as Fx
            };
        }
        v
    });
    Vertex {
        position,
        colour,
        normal,
        tex,
    }
}

/// The client arrays as a draw reads them: each one given while a buffer
/// object was bound read from that buffer's store (#1488).
fn resolved(c: &mut Context) -> Arrays {
    let mut a = c.arrays;
    for x in [&mut a.vertex, &mut a.colour, &mut a.normal, &mut a.texcoord] {
        x.at = c.buffers.resolve(x.buffer, x.at);
    }
    a
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
    let arrays = resolved(c);
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
        at: c.buffers.resolve(c.buffers.element, indices as *const u8),
        on: true,
        buffer: 0,
    };
    let arrays = resolved(c);
    c.gl.draw_vertices(mode, count as usize, |k| {
        fetch(&arrays, index.raw(k, 0) as usize)
    });
}

/// Buffer objects (#1488): the names a context can hold, GL ES 1.1's
/// targets, usages and queries.
const BUFFERS: usize = 64;
const ARRAY_BUFFER: u32 = 0x8892;
const ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
const STATIC_DRAW: u32 = 0x88E4;
const DYNAMIC_DRAW: u32 = 0x88E8;
const BUFFER_SIZE: u32 = 0x8764;
const BUFFER_USAGE: u32 = 0x8765;

/// A buffer object: whether its name is in use, where its store starts
/// in the context's room and how long it is, and its usage.
#[derive(Clone, Copy)]
struct Buffer {
    live: bool,
    at: usize,
    size: usize,
    usage: u32,
}

impl Buffer {
    const NONE: Buffer = Buffer {
        live: false,
        at: 0,
        size: 0,
        usage: STATIC_DRAW,
    };
}

/// A context's buffer objects: the room their stores live in, which the
/// machine gives as it gives the textures' (`gles_buffer_room`), handed
/// out in order and never given back, as the textures' is; the objects;
/// and the two bindings.
struct Buffers {
    room: *mut u8,
    len: usize,
    used: usize,
    objects: [Buffer; BUFFERS],
    array: u32,
    element: u32,
}

impl Buffers {
    const NONE: Buffers = Buffers {
        room: core::ptr::null_mut(),
        len: 0,
        used: 0,
        objects: [Buffer::NONE; BUFFERS],
        array: 0,
        element: 0,
    };

    /// The object called `name`, if it is in use.
    fn object(&mut self, name: u32) -> Option<&mut Buffer> {
        let o = self.objects.get_mut((name as usize).checked_sub(1)?)?;
        o.live.then_some(o)
    }

    /// Where an array or the indices start: `at` itself when no buffer
    /// was bound as they were given, or `at` bytes into that buffer's
    /// store, as GL has it.
    fn resolve(&mut self, buffer: u32, at: *const u8) -> *const u8 {
        let room = self.room;
        match self.object(buffer) {
            Some(o) if buffer != 0 && !room.is_null() => {
                // SAFETY: the store lies inside the room, and GL leaves an
                // offset past it to the caller.
                unsafe { room.add(o.at + at as usize) as *const u8 }
            }
            _ => at,
        }
    }

    /// The binding of `target`.
    fn bound(&mut self, target: u32) -> Option<&mut u32> {
        match target {
            ARRAY_BUFFER => Some(&mut self.array),
            ELEMENT_ARRAY_BUFFER => Some(&mut self.element),
            _ => None,
        }
    }
}

/// Gives the current context room for its buffer objects' stores (#1488):
/// `bytes` bytes at `mem`. Not a GL call: EGL's, at `eglMakeCurrent`.
/// Without it `glBufferData` fails with `GL_OUT_OF_MEMORY`.
///
/// # Safety
/// `mem` must be `bytes` bytes the context may keep for as long as it
/// lives, which nothing else writes.
#[no_mangle]
pub unsafe extern "C" fn gles_buffer_room(mem: *mut u8, bytes: usize) {
    if let Some(c) = current() {
        (c.buffers.room, c.buffers.len, c.buffers.used) = (mem, bytes, 0);
    }
}

#[no_mangle]
pub unsafe extern "C" fn glGenBuffers(n: i32, out: *mut u32) {
    let Some(c) = current() else {
        return;
    };
    if n < 0 {
        return c.gl.record_error(gl::INVALID_VALUE);
    }
    let objects = &mut c.buffers.objects;
    let free = objects.iter().filter(|o| !o.live).count();
    if (n as usize) > free {
        return c.gl.record_error(gl::OUT_OF_MEMORY);
    }
    let mut k = 0;
    for (i, o) in objects.iter_mut().enumerate() {
        if k == n as usize {
            break;
        }
        if !o.live {
            *o = Buffer {
                live: true,
                ..Buffer::NONE
            };
            *out.add(k) = i as u32 + 1;
            k += 1;
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn glDeleteBuffers(n: i32, names: *const u32) {
    let Some(c) = current() else {
        return;
    };
    if n < 0 {
        return c.gl.record_error(gl::INVALID_VALUE);
    }
    for k in 0..n as usize {
        let name = *names.add(k);
        if let Some(o) = c.buffers.object(name) {
            o.live = false;
            // A deleted buffer is unbound wherever it was bound.
            for b in [&mut c.buffers.array, &mut c.buffers.element] {
                if *b == name {
                    *b = 0;
                }
            }
            let a = &mut c.arrays;
            for x in
                [&mut a.vertex, &mut a.colour, &mut a.normal, &mut a.texcoord]
            {
                if x.buffer == name {
                    x.buffer = 0;
                }
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn glIsBuffer(name: u32) -> u8 {
    current().map_or(0, |c| c.buffers.object(name).is_some() as u8)
}

#[no_mangle]
pub extern "C" fn glBindBuffer(target: u32, name: u32) {
    let Some(c) = current() else {
        return;
    };
    // GL ES 1.1 makes an object of a name not yet in use as it is bound.
    let made = match (name as usize).checked_sub(1) {
        Some(i) if i < BUFFERS => {
            let o = &mut c.buffers.objects[i];
            if !o.live {
                *o = Buffer {
                    live: true,
                    ..Buffer::NONE
                };
            }
            true
        }
        Some(_) => false,
        None => true,
    };
    if !made {
        return c.gl.record_error(gl::OUT_OF_MEMORY);
    }
    match c.buffers.bound(target) {
        Some(b) => *b = name,
        None => c.gl.record_error(gl::INVALID_ENUM),
    }
}

#[no_mangle]
pub unsafe extern "C" fn glBufferData(
    target: u32,
    size: isize,
    data: *const c_void,
    usage: u32,
) {
    let Some(c) = current() else {
        return;
    };
    if size < 0 {
        return c.gl.record_error(gl::INVALID_VALUE);
    }
    if usage != STATIC_DRAW && usage != DYNAMIC_DRAW {
        return c.gl.record_error(gl::INVALID_ENUM);
    }
    let name = match c.buffers.bound(target) {
        Some(&mut n) => n,
        None => return c.gl.record_error(gl::INVALID_ENUM),
    };
    let (room, len, used) = (c.buffers.room, c.buffers.len, c.buffers.used);
    let at = used.div_ceil(4) * 4;
    let fits = !room.is_null() && at + size as usize <= len;
    let Some(o) = c.buffers.object(name) else {
        return c.gl.record_error(gl::INVALID_OPERATION);
    };
    if !fits {
        return c.gl.record_error(gl::OUT_OF_MEMORY);
    }
    (o.at, o.size, o.usage) = (at, size as usize, usage);
    if !data.is_null() {
        core::ptr::copy_nonoverlapping(
            data as *const u8,
            room.add(at),
            size as usize,
        );
    }
    c.buffers.used = at + size as usize;
}

#[no_mangle]
pub unsafe extern "C" fn glBufferSubData(
    target: u32,
    offset: isize,
    size: isize,
    data: *const c_void,
) {
    let Some(c) = current() else {
        return;
    };
    let name = match c.buffers.bound(target) {
        Some(&mut n) => n,
        None => return c.gl.record_error(gl::INVALID_ENUM),
    };
    let room = c.buffers.room;
    let Some(o) = c.buffers.object(name) else {
        return c.gl.record_error(gl::INVALID_OPERATION);
    };
    let inside = offset >= 0 && size >= 0 && (offset + size) as usize <= o.size;
    if !inside || data.is_null() {
        return c.gl.record_error(gl::INVALID_VALUE);
    }
    core::ptr::copy_nonoverlapping(
        data as *const u8,
        room.add(o.at + offset as usize),
        size as usize,
    );
}

#[no_mangle]
pub unsafe extern "C" fn glGetBufferParameteriv(
    target: u32,
    pname: u32,
    out: *mut i32,
) {
    let Some(c) = current() else {
        return;
    };
    let name = match c.buffers.bound(target) {
        Some(&mut n) => n,
        None => return c.gl.record_error(gl::INVALID_ENUM),
    };
    let Some(o) = c.buffers.object(name) else {
        return c.gl.record_error(gl::INVALID_OPERATION);
    };
    let v = match pname {
        BUFFER_SIZE => o.size as i32,
        BUFFER_USAGE => o.usage as i32,
        _ => return c.gl.record_error(gl::INVALID_ENUM),
    };
    if !out.is_null() {
        *out = v;
    }
}

/// Gives the current context room for its textures (#997): `words` words
/// at `mem`, which Razboj reads at the bus address `bus`. Not a GL call:
/// EGL's, at `eglMakeCurrent`.
///
/// # Safety
/// `mem` must be `words` words the context may keep for as long as it
/// lives, which nothing else writes.
#[no_mangle]
pub unsafe extern "C" fn gles_texture_room(
    mem: *mut u32,
    words: usize,
    bus: u32,
) {
    let room = core::slice::from_raw_parts_mut(mem, words);
    with(|g| g.texture_room(room, bus));
}

#[no_mangle]
pub extern "C" fn glTexCoordPointer(
    size: i32,
    kind: u32,
    stride: i32,
    at: *const c_void,
) {
    pointer(
        gl::TEXTURE_COORD_ARRAY,
        size,
        kind,
        stride,
        at,
        &[2, 3, 4],
        &[BYTE, SHORT, FIXED],
    );
}

#[no_mangle]
pub unsafe extern "C" fn glGenTextures(n: i32, names: *mut u32) {
    if n < 0 {
        return gles_record_error(gl::INVALID_VALUE);
    }
    let out = core::slice::from_raw_parts_mut(names, n as usize);
    with(|g| g.gen_textures(out));
}

#[no_mangle]
pub unsafe extern "C" fn glDeleteTextures(n: i32, names: *const u32) {
    if n < 0 {
        return gles_record_error(gl::INVALID_VALUE);
    }
    let names = core::slice::from_raw_parts(names, n as usize);
    with(|g| g.delete_textures(names));
}

#[no_mangle]
pub extern "C" fn glBindTexture(target: u32, name: u32) {
    with(|g| g.bind_texture(target, name));
}

/// The bytes `glTexImage2D` reads: every row but the last padded to the
/// unpack alignment `align`, as the library reads them.
fn image_bytes(
    w: usize,
    h: usize,
    format: u32,
    kind: u32,
    align: usize,
) -> usize {
    let texel = match (format, kind) {
        (gl::RGBA, gl::UNSIGNED_BYTE) => 4,
        (gl::RGB, gl::UNSIGNED_BYTE) => 3,
        (gl::LUMINANCE_ALPHA, gl::UNSIGNED_BYTE) => 2,
        (_, gl::UNSIGNED_BYTE) => 1,
        _ => 2,
    };
    (w * texel).div_ceil(align) * align * (h - 1) + w * texel
}

#[no_mangle]
pub unsafe extern "C" fn glTexImage2D(
    target: u32,
    level: i32,
    internal: i32,
    width: i32,
    height: i32,
    border: i32,
    format: u32,
    kind: u32,
    pixels: *const c_void,
) {
    if level < 0 || width < 1 || height < 1 || border != 0 || pixels.is_null() {
        return gles_record_error(gl::INVALID_VALUE);
    }
    let (w, h) = (width as usize, height as usize);
    let Some(c) = current() else {
        return;
    };
    let bytes = image_bytes(w, h, format, kind, c.gl.unpack_alignment());
    let data = core::slice::from_raw_parts(pixels as *const u8, bytes);
    with(|g| {
        g.tex_image_2d(
            target,
            level as u32,
            internal as u32,
            w as u32,
            h as u32,
            0,
            format,
            kind,
            data,
        )
    });
}

/// `glCompressedTexImage2D`, the paletted formats only (#998): `size`
/// bytes from `data`, the palette and the levels' indices.
#[no_mangle]
pub unsafe extern "C" fn glCompressedTexImage2D(
    target: u32,
    level: i32,
    internal: u32,
    width: i32,
    height: i32,
    border: i32,
    size: i32,
    data: *const c_void,
) {
    if width < 1 || height < 1 || border != 0 || size < 0 || data.is_null() {
        return gles_record_error(gl::INVALID_VALUE);
    }
    let bytes = core::slice::from_raw_parts(data as *const u8, size as usize);
    with(|g| {
        g.compressed_tex_image_2d(
            target,
            level,
            internal,
            width as u32,
            height as u32,
            0,
            bytes,
        )
    });
}

#[no_mangle]
pub extern "C" fn glTexParameteri(target: u32, pname: u32, param: i32) {
    with(|g| g.tex_parameter(target, pname, param as u32));
}

/// An enumerant given through the fixed-point call is the enumerant's
/// own value, as GL ES 1.1 says of enumerated parameters.
#[no_mangle]
pub extern "C" fn glTexParameterx(target: u32, pname: u32, param: Fx) {
    with(|g| g.tex_parameter(target, pname, param as u32));
}

#[no_mangle]
pub extern "C" fn glTexEnvx(target: u32, pname: u32, param: Fx) {
    with(|g| g.tex_env(target, pname, &[param]));
}

#[no_mangle]
pub extern "C" fn glTexEnvi(target: u32, pname: u32, param: i32) {
    with(|g| g.tex_env(target, pname, &[param as Fx]));
}

#[no_mangle]
pub unsafe extern "C" fn glTexEnvxv(
    target: u32,
    pname: u32,
    params: *const Fx,
) {
    let n = if pname == gl::TEXTURE_ENV_COLOR { 4 } else { 1 };
    let v = vals(params, n);
    with(|g| g.tex_env(target, pname, &v[..n]));
}

/// The one texture unit, `GL_TEXTURE0`: any other is
/// `GL_INVALID_ENUM`.
const TEXTURE0: u32 = 0x84C0;

#[no_mangle]
pub extern "C" fn glActiveTexture(unit: u32) {
    if unit != TEXTURE0 {
        gles_record_error(gl::INVALID_ENUM);
    }
}

#[no_mangle]
pub extern "C" fn glClientActiveTexture(unit: u32) {
    glActiveTexture(unit);
}

#[no_mangle]
pub extern "C" fn glMultiTexCoord4x(unit: u32, s: Fx, t: Fx, r: Fx, q: Fx) {
    if unit != TEXTURE0 {
        return gles_record_error(gl::INVALID_ENUM);
    }
    with(|g| g.tex_coord(s, t, r, q));
}

#[no_mangle]
pub extern "C" fn glPixelStorei(pname: u32, param: i32) {
    with(|g| g.pixel_store(pname, param as u32));
}
