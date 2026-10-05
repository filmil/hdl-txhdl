// SPDX-License-Identifier: Apache-2.0
//! GL, EGL and the board's machine as one static library for a Zephyr
//! program on Vreteno (issue 996): the root that brings GL's and EGL's
//! entry points with it, installs the board's machine for EGL from C,
//! and is the one place a panic handler is, as a `no_std` library a C
//! program links must have.
#![no_std]

// GL's and EGL's entry points, which the library exports.
pub use gles_capi;
pub use gles_egl;

use gles_vreteno::Board;

static mut BOARD: Board = Board::new();

/// Installs the board's machine for EGL: what a program on the board
/// calls once before `eglInitialize`.
#[no_mangle]
pub extern "C" fn egl_vreteno_install() {
    // SAFETY: one core, and called once, before EGL is used.
    gles_egl::install(unsafe { &mut *core::ptr::addr_of_mut!(BOARD) });
}

/// Nothing in the library panics by design; one that did would be a
/// bug, and stops here rather than running on in a broken state.
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
