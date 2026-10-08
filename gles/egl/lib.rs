// SPDX-License-Identifier: Apache-2.0
//! EGL for Razboj (issue 996): the calls Khronos's `EGL/egl.h` declares
//! that a GL ES 1.1 program makes to get a window and a context and to
//! show what it drew, as `extern "C"` functions over the GL library's C
//! entry points. Rust inside with C at the edge, as the GL library is.
//!
//! ## The window and its two buffers
//!
//! There is one display, one configuration and one window, 640 by 480:
//! what the scanout shows. Razboj's framebuffer has rows of 1024 words
//! from `0x4200_0000`, and the window is double buffered in it, rows 0
//! to 479 and rows 512 to 991, as the icosahedron is (issue 986). GL draws
//! into the buffer not shown: EGL retargets the context there, so the
//! library clips to that buffer's rows and clears only them, and nothing
//! ever lands on the buffer being shown.
//!
//! ## A swap
//!
//! `eglSwapBuffers` hands the frame GL wrote straight into Razboj's
//! display list to the rasteriser and waits until every pixel of it is
//! written; points the scanout at that buffer; and waits for the
//! vertical blanking, where the scanout takes its base. Then the other
//! buffer is the one drawn into. A frame is therefore never shown half
//! drawn, and the swap interval is one, always.
//!
//! A frame that tests depth anywhere is not a flat list, since Razboj
//! tests depth only in a tile table (#992, #1273). The swap bins it into
//! the machine's scratch room and has the machine draw that as a tile
//! table, laid out at the list. Too little room draws it flat, its depth
//! untested, and fails the swap with `EGL_BAD_ALLOC`.
//!
//! ## The machine
//!
//! What a swap does to the hardware is behind [`Machine`]: the display
//! list's memory, the doorbell, the scanout's base and the blanking. The
//! board's is in `egl_vreteno`; a test gives one of its own, which draws
//! through Razboj's model, and [`install`] sets which. A call with no
//! machine installed fails with `EGL_NOT_INITIALIZED`.
//!
//! The calls `egl.h` declares that this does not implement are written
//! from the header by `capi/stubs.sh`, and fail with `EGL_BAD_MATCH`.
#![cfg_attr(not(feature = "std"), no_std)]
// The entry points take the caller's pointers, as EGL's do; their
// validity is the caller's, and each is read only where EGL reads it.
#![allow(clippy::missing_safety_doc)]

use core::ffi::c_void;
use gles_capi::{
    gles_buffer_room, gles_flush, gles_frame_len, gles_frame_tiled,
    gles_make_current, gles_retarget, gles_texture_room,
};
use razboj_tile::{MAX_TILES, TILE_WORDS};

/// EGL's types as `egl.h` has them, with no window system's.
pub type EGLBoolean = u32;
pub type EGLint = i32;
pub type EGLenum = u32;
pub type Handle = *mut c_void;

const TRUE: EGLBoolean = 1;
const FALSE: EGLBoolean = 0;

/// `egl.h`'s errors.
pub const SUCCESS: EGLint = 0x3000;
pub const NOT_INITIALIZED: EGLint = 0x3001;
pub const BAD_ACCESS: EGLint = 0x3002;
pub const BAD_ALLOC: EGLint = 0x3003;
pub const BAD_ATTRIBUTE: EGLint = 0x3004;
pub const BAD_CONFIG: EGLint = 0x3005;
pub const BAD_CONTEXT: EGLint = 0x3006;
pub const BAD_DISPLAY: EGLint = 0x3008;
pub const BAD_MATCH: EGLint = 0x3009;
pub const BAD_PARAMETER: EGLint = 0x300C;
pub const BAD_SURFACE: EGLint = 0x300D;

/// The attributes and names it reads and answers.
const BUFFER_SIZE: EGLint = 0x3020;
const ALPHA_SIZE: EGLint = 0x3021;
const BLUE_SIZE: EGLint = 0x3022;
const GREEN_SIZE: EGLint = 0x3023;
const RED_SIZE: EGLint = 0x3024;
const DEPTH_SIZE: EGLint = 0x3025;
const STENCIL_SIZE: EGLint = 0x3026;
const CONFIG_CAVEAT: EGLint = 0x3027;
const CONFIG_ID: EGLint = 0x3028;
const LEVEL: EGLint = 0x3029;
const NATIVE_RENDERABLE: EGLint = 0x302D;
const SAMPLES: EGLint = 0x3031;
const SAMPLE_BUFFERS: EGLint = 0x3032;
const SURFACE_TYPE: EGLint = 0x3033;
const TRANSPARENT_TYPE: EGLint = 0x3034;
const NONE: EGLint = 0x3038;
const MIN_SWAP_INTERVAL: EGLint = 0x303B;
const MAX_SWAP_INTERVAL: EGLint = 0x303C;
const COLOR_BUFFER_TYPE: EGLint = 0x303F;
const RENDERABLE_TYPE: EGLint = 0x3040;
const CONFORMANT: EGLint = 0x3042;
const RGB_BUFFER: EGLint = 0x308E;
const WINDOW_BIT: EGLint = 0x0004;
const OPENGL_ES_BIT: EGLint = 0x0001;
const VENDOR: EGLint = 0x3053;
const VERSION: EGLint = 0x3054;
const EXTENSIONS: EGLint = 0x3055;
const CLIENT_APIS: EGLint = 0x308D;
const HEIGHT: EGLint = 0x3056;
const WIDTH: EGLint = 0x3057;
const DRAW: EGLint = 0x3059;
const READ: EGLint = 0x305A;
const CONTEXT_CLIENT_VERSION: EGLint = 0x3098;
const OPENGL_ES_API: EGLenum = 0x30A0;
const DONT_CARE: EGLint = -1;

/// The window, and where its two buffers start in Razboj's rows.
pub const WINDOW_WIDTH: u32 = 640;
pub const WINDOW_HEIGHT: u32 = 480;
pub const BUFFER_ROWS: [u32; 2] = [0, 512];

/// What a swap needs of the machine, from its own crate.
pub use gles_machine::Machine;

/// EGL's state: the machine, whether the display is initialised, the
/// error, the window and the context made and current, and which buffer
/// is drawn into.
struct State {
    machine: Option<&'static mut dyn Machine>,
    initialised: bool,
    error: EGLint,
    surface: bool,
    context: bool,
    current: bool,
    back: usize,
}

static mut STATE: State = State {
    machine: None,
    initialised: false,
    error: SUCCESS,
    surface: false,
    context: false,
    current: false,
    back: 1,
};

fn state() -> &'static mut State {
    // SAFETY: one core and no threads, as GL's context is.
    unsafe { &mut *core::ptr::addr_of_mut!(STATE) }
}

/// Sets the machine EGL drives, and starts from nothing made.
pub fn install(machine: &'static mut dyn Machine) {
    let s = state();
    *s = State {
        machine: Some(machine),
        initialised: false,
        error: SUCCESS,
        surface: false,
        context: false,
        current: false,
        back: 1,
    };
}

/// The one display, configuration, window and context, as handles.
const DISPLAY: Handle = 1 as Handle;
const CONFIG: Handle = 1 as Handle;
const SURFACE: Handle = 1 as Handle;
const CONTEXT: Handle = 1 as Handle;
const NO: Handle = core::ptr::null_mut();

/// Fails with `error`, which `eglGetError` reports next.
fn fail<T>(error: EGLint, value: T) -> T {
    state().error = error;
    value
}

/// Succeeds with `value`.
fn ok<T>(value: T) -> T {
    state().error = SUCCESS;
    value
}

/// The display, initialised, or the error EGL gives.
fn display(dpy: Handle) -> Result<(), EGLint> {
    let s = state();
    if dpy != DISPLAY {
        return Err(BAD_DISPLAY);
    }
    if !s.initialised || s.machine.is_none() {
        return Err(NOT_INITIALIZED);
    }
    Ok(())
}

/// Records `error` for `eglGetError`, as a call this does not implement
/// does (`stubs.c`).
#[no_mangle]
pub extern "C" fn egl_record_error(error: EGLint) {
    state().error = error;
}

#[no_mangle]
pub extern "C" fn eglGetError() -> EGLint {
    core::mem::replace(&mut state().error, SUCCESS)
}

#[no_mangle]
pub extern "C" fn eglGetDisplay(_native: Handle) -> Handle {
    DISPLAY
}

#[no_mangle]
pub unsafe extern "C" fn eglInitialize(
    dpy: Handle,
    major: *mut EGLint,
    minor: *mut EGLint,
) -> EGLBoolean {
    let s = state();
    if dpy != DISPLAY {
        return fail(BAD_DISPLAY, FALSE);
    }
    if s.machine.is_none() {
        return fail(NOT_INITIALIZED, FALSE);
    }
    s.initialised = true;
    if !major.is_null() {
        *major = 1;
    }
    if !minor.is_null() {
        *minor = 4;
    }
    ok(TRUE)
}

#[no_mangle]
pub extern "C" fn eglTerminate(dpy: Handle) -> EGLBoolean {
    if dpy != DISPLAY {
        return fail(BAD_DISPLAY, FALSE);
    }
    let s = state();
    s.initialised = false;
    s.surface = false;
    s.context = false;
    s.current = false;
    ok(TRUE)
}

/// The one configuration's value of `attribute`, if it has one.
fn attribute(attribute: EGLint) -> Option<EGLint> {
    Some(match attribute {
        BUFFER_SIZE => 32,
        RED_SIZE | GREEN_SIZE | BLUE_SIZE | ALPHA_SIZE => 8,
        // The depth of a tile table (#1273), as GL_DEPTH_BITS says (#1502).
        DEPTH_SIZE => 16,
        // The stencil, beside each depth in the tile (#998).
        STENCIL_SIZE => 8,
        SAMPLES | SAMPLE_BUFFERS | LEVEL => 0,
        CONFIG_CAVEAT | TRANSPARENT_TYPE => NONE,
        CONFIG_ID => 1,
        NATIVE_RENDERABLE => FALSE as EGLint,
        SURFACE_TYPE => WINDOW_BIT,
        RENDERABLE_TYPE | CONFORMANT => OPENGL_ES_BIT,
        MIN_SWAP_INTERVAL | MAX_SWAP_INTERVAL => 1,
        COLOR_BUFFER_TYPE => RGB_BUFFER,
        _ => return None,
    })
}

/// Whether the configuration meets an asked-for `value` of `name`:
/// sizes at least, bit masks all present, the rest exactly.
fn meets(name: EGLint, value: EGLint) -> Result<bool, EGLint> {
    if value == DONT_CARE {
        return Ok(true);
    }
    let have = attribute(name).ok_or(BAD_ATTRIBUTE)?;
    Ok(match name {
        BUFFER_SIZE | RED_SIZE | GREEN_SIZE | BLUE_SIZE | ALPHA_SIZE
        | DEPTH_SIZE | STENCIL_SIZE | SAMPLES | SAMPLE_BUFFERS => have >= value,
        SURFACE_TYPE | RENDERABLE_TYPE | CONFORMANT => have & value == value,
        _ => have == value,
    })
}

#[no_mangle]
pub unsafe extern "C" fn eglChooseConfig(
    dpy: Handle,
    attribs: *const EGLint,
    configs: *mut Handle,
    size: EGLint,
    count: *mut EGLint,
) -> EGLBoolean {
    if let Err(e) = display(dpy) {
        return fail(e, FALSE);
    }
    if count.is_null() {
        return fail(BAD_PARAMETER, FALSE);
    }
    let mut fits = true;
    let mut at = attribs;
    while !at.is_null() && *at != NONE {
        match meets(*at, *at.add(1)) {
            Ok(m) => fits &= m,
            Err(e) => return fail(e, FALSE),
        }
        at = at.add(2);
    }
    let n = fits as EGLint;
    if !configs.is_null() && size > 0 && fits {
        *configs = CONFIG;
    }
    *count = n;
    ok(TRUE)
}

#[no_mangle]
pub unsafe extern "C" fn eglGetConfigs(
    dpy: Handle,
    configs: *mut Handle,
    size: EGLint,
    count: *mut EGLint,
) -> EGLBoolean {
    if let Err(e) = display(dpy) {
        return fail(e, FALSE);
    }
    if count.is_null() {
        return fail(BAD_PARAMETER, FALSE);
    }
    if !configs.is_null() && size > 0 {
        *configs = CONFIG;
    }
    *count = 1;
    ok(TRUE)
}

#[no_mangle]
pub unsafe extern "C" fn eglGetConfigAttrib(
    dpy: Handle,
    config: Handle,
    name: EGLint,
    value: *mut EGLint,
) -> EGLBoolean {
    if let Err(e) = display(dpy) {
        return fail(e, FALSE);
    }
    if config != CONFIG {
        return fail(BAD_CONFIG, FALSE);
    }
    match attribute(name) {
        Some(v) if !value.is_null() => {
            *value = v;
            ok(TRUE)
        }
        _ => fail(BAD_ATTRIBUTE, FALSE),
    }
}

#[no_mangle]
pub extern "C" fn eglCreateWindowSurface(
    dpy: Handle,
    config: Handle,
    _window: Handle,
    _attribs: *const EGLint,
) -> Handle {
    if let Err(e) = display(dpy) {
        return fail(e, NO);
    }
    if config != CONFIG {
        return fail(BAD_CONFIG, NO);
    }
    let s = state();
    if s.surface {
        return fail(BAD_MATCH, NO);
    }
    s.surface = true;
    ok(SURFACE)
}

#[no_mangle]
pub extern "C" fn eglDestroySurface(
    dpy: Handle,
    surface: Handle,
) -> EGLBoolean {
    if let Err(e) = display(dpy) {
        return fail(e, FALSE);
    }
    let s = state();
    if surface != SURFACE || !s.surface {
        return fail(BAD_SURFACE, FALSE);
    }
    s.surface = false;
    s.current = false;
    ok(TRUE)
}

#[no_mangle]
pub unsafe extern "C" fn eglQuerySurface(
    dpy: Handle,
    surface: Handle,
    name: EGLint,
    value: *mut EGLint,
) -> EGLBoolean {
    if let Err(e) = display(dpy) {
        return fail(e, FALSE);
    }
    if surface != SURFACE || !state().surface {
        return fail(BAD_SURFACE, FALSE);
    }
    let v = match name {
        WIDTH => WINDOW_WIDTH as EGLint,
        HEIGHT => WINDOW_HEIGHT as EGLint,
        CONFIG_ID => 1,
        _ => return fail(BAD_ATTRIBUTE, FALSE),
    };
    if !value.is_null() {
        *value = v;
    }
    ok(TRUE)
}

#[no_mangle]
pub extern "C" fn eglBindAPI(api: EGLenum) -> EGLBoolean {
    if api == OPENGL_ES_API {
        ok(TRUE)
    } else {
        fail(BAD_PARAMETER, FALSE)
    }
}

#[no_mangle]
pub extern "C" fn eglQueryAPI() -> EGLenum {
    OPENGL_ES_API
}

#[no_mangle]
pub unsafe extern "C" fn eglCreateContext(
    dpy: Handle,
    config: Handle,
    _share: Handle,
    attribs: *const EGLint,
) -> Handle {
    if let Err(e) = display(dpy) {
        return fail(e, NO);
    }
    if config != CONFIG {
        return fail(BAD_CONFIG, NO);
    }
    // Only GL ES 1.x: a context of version 2 or more is not this one.
    let mut at = attribs;
    while !at.is_null() && *at != NONE {
        if *at == CONTEXT_CLIENT_VERSION && *at.add(1) != 1 {
            return fail(BAD_MATCH, NO);
        }
        at = at.add(2);
    }
    let s = state();
    if s.context {
        return fail(BAD_MATCH, NO);
    }
    s.context = true;
    ok(CONTEXT)
}

#[no_mangle]
pub extern "C" fn eglDestroyContext(
    dpy: Handle,
    context: Handle,
) -> EGLBoolean {
    if let Err(e) = display(dpy) {
        return fail(e, FALSE);
    }
    let s = state();
    if context != CONTEXT || !s.context {
        return fail(BAD_CONTEXT, FALSE);
    }
    s.context = false;
    s.current = false;
    ok(TRUE)
}

/// Points the current GL context at the buffer drawn into, empty.
fn target(machine: &mut dyn Machine, back: usize) {
    let list = machine.list();
    // SAFETY: the list is the machine's for good, and GL is its only
    // writer until the next swap hands it to Razboj.
    unsafe {
        gles_retarget(
            list.as_mut_ptr() as *mut u32,
            list.len(),
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            BUFFER_ROWS[back],
        )
    };
}

#[no_mangle]
pub extern "C" fn eglMakeCurrent(
    dpy: Handle,
    draw: Handle,
    read: Handle,
    context: Handle,
) -> EGLBoolean {
    if let Err(e) = display(dpy) {
        return fail(e, FALSE);
    }
    let s = state();
    if context == NO && draw == NO && read == NO {
        s.current = false;
        return ok(TRUE);
    }
    if context != CONTEXT || !s.context {
        return fail(BAD_CONTEXT, FALSE);
    }
    if draw != SURFACE || read != SURFACE || !s.surface {
        return fail(BAD_SURFACE, FALSE);
    }
    if !s.current {
        let Some(m) = s.machine.as_deref_mut() else {
            return fail(NOT_INITIALIZED, FALSE);
        };
        let list = m.list();
        // SAFETY: as in `target`.
        unsafe {
            gles_make_current(
                list.as_mut_ptr() as *mut u32,
                list.len(),
                WINDOW_WIDTH,
                WINDOW_HEIGHT,
            )
        };
        if let Some((room, bus)) = m.textures() {
            // SAFETY: the machine's room is its own and static, as the
            // list is.
            unsafe { gles_texture_room(room.as_mut_ptr(), room.len(), bus) };
        }
        if let Some(room) = m.buffers() {
            // SAFETY: as the textures' room.
            unsafe { gles_buffer_room(room.as_mut_ptr(), room.len()) };
        }
        target(m, s.back);
        s.current = true;
    }
    ok(TRUE)
}

#[no_mangle]
pub extern "C" fn eglGetCurrentContext() -> Handle {
    if state().current {
        CONTEXT
    } else {
        NO
    }
}

#[no_mangle]
pub extern "C" fn eglGetCurrentSurface(which: EGLint) -> Handle {
    if state().current && (which == DRAW || which == READ) {
        SURFACE
    } else {
        NO
    }
}

#[no_mangle]
pub extern "C" fn eglGetCurrentDisplay() -> Handle {
    if state().current {
        DISPLAY
    } else {
        NO
    }
}

/// The frame drawn into the buffer not shown, drawn by Razboj, shown
/// from the next vertical blanking; then the other buffer is drawn into.
#[no_mangle]
pub extern "C" fn eglSwapBuffers(dpy: Handle, surface: Handle) -> EGLBoolean {
    if let Err(e) = display(dpy) {
        return fail(e, FALSE);
    }
    let s = state();
    if surface != SURFACE || !s.surface {
        return fail(BAD_SURFACE, FALSE);
    }
    if !s.current {
        return fail(BAD_CONTEXT, FALSE);
    }
    let Some(m) = s.machine.as_deref_mut() else {
        return fail(NOT_INITIALIZED, FALSE);
    };
    // A frame that tests depth is drawn from a tile table, which only
    // tests it (#1273); one that does not, as the flat list it is. Too
    // little room to bin it in draws it flat, its depth untested, and
    // says so.
    let mut drawn = true;
    if gles_frame_tiled() {
        let mut tiles = [[0u32; TILE_WORDS]; MAX_TILES];
        let room = m.scratch();
        match gles_flush(room, &mut tiles) {
            Some(b) => m.draw_tiled(&tiles[..b.tiles], &room[..b.entries]),
            None => {
                m.draw(gles_frame_len());
                drawn = false;
            }
        }
    } else {
        m.draw(gles_frame_len());
    }
    m.show(BUFFER_ROWS[s.back]);
    m.wait_blanking();
    s.back ^= 1;
    target(m, s.back);
    if drawn {
        ok(TRUE)
    } else {
        fail(BAD_ALLOC, FALSE)
    }
}

/// The interval is one: every swap waits for the blanking, which is
/// what keeps a frame from being shown half drawn.
#[no_mangle]
pub extern "C" fn eglSwapInterval(
    dpy: Handle,
    _interval: EGLint,
) -> EGLBoolean {
    if let Err(e) = display(dpy) {
        return fail(e, FALSE);
    }
    ok(TRUE)
}

/// GL's calls are drawn at the swap, so there is nothing to wait for.
#[no_mangle]
pub extern "C" fn eglWaitClient() -> EGLBoolean {
    ok(TRUE)
}

#[no_mangle]
pub extern "C" fn eglWaitGL() -> EGLBoolean {
    ok(TRUE)
}

#[no_mangle]
pub extern "C" fn eglWaitNative(_engine: EGLint) -> EGLBoolean {
    ok(TRUE)
}

#[no_mangle]
pub extern "C" fn eglReleaseThread() -> EGLBoolean {
    state().current = false;
    ok(TRUE)
}

#[no_mangle]
pub extern "C" fn eglQueryString(dpy: Handle, name: EGLint) -> *const u8 {
    if let Err(e) = display(dpy) {
        return fail(e, core::ptr::null());
    }
    let s: &'static [u8] = match name {
        VENDOR => b"TxHDL\0",
        VERSION => b"1.4 TxHDL Razboj\0",
        CLIENT_APIS => b"OpenGL_ES\0",
        EXTENSIONS => b"\0",
        _ => return fail(BAD_PARAMETER, core::ptr::null()),
    };
    ok(s.as_ptr())
}
