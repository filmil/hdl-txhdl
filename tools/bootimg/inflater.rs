// SPDX-License-Identifier: Apache-2.0
//! The boot image's inflater, on the core (issue 1229): the kernel,
//! raw DEFLATE where the shim moved it, inflated to its load address.
//!
//! The shim calls it as a C function at its first byte, `INFLATER` in
//! `bootimg`, with a stack in the core's data memory: `a0` the stream
//! and `a1` its length, `a2` the kernel's address and `a3` its length.
//! It answers in `a0`: nought when exactly that many bytes came out,
//! one when another number did, and two and up for the stream's error.
//! It has no data and no BSS, which the linker script holds it to,
//! since nothing sets them up before the shim calls it.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {}
}

/// The entry, at the image's first byte.
///
/// # Safety
///
/// The shim passes ranges of memory that are the stream's and the
/// kernel's, which nothing else uses while it runs.
#[no_mangle]
#[link_section = ".text.init"]
pub unsafe extern "C" fn _start(
    src: *const u8,
    len: usize,
    dst: *mut u8,
    cap: usize,
) -> usize {
    let s = unsafe { core::slice::from_raw_parts(src, len) };
    let d = unsafe { core::slice::from_raw_parts_mut(dst, cap) };
    match inflate::inflate(s, d) {
        Ok(n) if n == cap => 0,
        Ok(_) => 1,
        Err(e) => 2 + e as usize,
    }
}
