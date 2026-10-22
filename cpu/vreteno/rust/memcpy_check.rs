// SPDX-License-Identifier: Apache-2.0
//! The HAL's `memcpy` against a byte loop (#1825): every length from 0
//! to 100 bytes, from and to each of the four offsets in a word, into a
//! buffer whose bytes around the copy must stay as they were, and the
//! destination returned. `//cpu/vreteno:memcpy_test` runs it on the
//! machine.
//!
//! It says `memcpy ok <cases>`, or the first case that fails as
//! `memcpy FAIL <from> <to> <len>`, and halts.
#![no_std]
#![no_main]

use vreteno_hal::{entry, halt, Uart};

extern "C" {
    fn memcpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8;
}

const MAX: usize = 100;
const SIZE: usize = MAX + 16;

/// Word aligned, so that an offset of 0 is an aligned end.
#[repr(align(4))]
struct Buf([u8; SIZE]);

static mut SRC: Buf = Buf([0; SIZE]);
static mut DST: Buf = Buf([0; SIZE]);

/// A byte no two neighbours share, for a case's from and its guard.
fn pattern(k: usize, seed: usize) -> u8 {
    (k.wrapping_mul(37) ^ seed.wrapping_mul(101)) as u8 | 1
}

fn check(from: usize, to: usize, len: usize) -> bool {
    let (src, dst) = unsafe {
        (
            &mut *core::ptr::addr_of_mut!(SRC.0),
            &mut *core::ptr::addr_of_mut!(DST.0),
        )
    };
    for k in 0..SIZE {
        src[k] = pattern(k, len);
        dst[k] = !pattern(k, len + 1);
    }
    let d = dst.as_mut_ptr().wrapping_add(to);
    let s = src.as_ptr().wrapping_add(from);
    let back = unsafe { memcpy(d, s, core::hint::black_box(len)) };
    if back != d {
        return false;
    }
    (0..SIZE).all(|k| {
        let want = if k >= to && k < to + len {
            src[from + k - to]
        } else {
            !pattern(k, len + 1)
        };
        dst[k] == want
    })
}

fn main() -> ! {
    let mut cases = 0u32;
    for from in 0..4 {
        for to in 0..4 {
            for len in 0..=MAX {
                if !check(from, to, len) {
                    Uart::say(b"memcpy FAIL ");
                    for v in [from, to] {
                        Uart::put_decimal(v as u32);
                        Uart::put(b' ');
                    }
                    Uart::put_decimal(len as u32);
                    Uart::put(b'\n');
                    halt()
                }
                cases += 1;
            }
        }
    }
    Uart::say(b"memcpy ok ");
    Uart::put_decimal(cases);
    Uart::put(b'\n');
    halt()
}

entry!(main);
