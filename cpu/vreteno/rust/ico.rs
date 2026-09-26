// SPDX-License-Identifier: Apache-2.0
//! An icosahedron, worked out by the core and filled by Razboj, the GPU.
//!
//! This is the software half of a display list. The core holds the
//! solid as twelve vertices and twenty triangles, turns it to face the
//! camera, works out which faces point at the camera and how brightly
//! each catches the light, and writes what is left into memory as the
//! words the rasteriser reads. It never touches a pixel: the
//! rasteriser finds the list where the program left it and fills
//! every triangle in it.
//!
//! The solid is convex, and that is why there is no sort. Of a convex
//! solid, the faces that point at the camera never cover one another,
//! so the order they are drawn in does not matter and the rasteriser
//! needs no depth buffer and the program no painter's algorithm.
//!
//! What the machine does not give you shapes the file, as it does in
//! `hello.rs`. There is no operating system, no standard library and
//! no floating point, so the geometry is integer throughout: the view
//! is a rotation matrix in eight fractional bits, worked out once
//! here rather than from a sine table on the core, and the
//! perspective is one division per coordinate.
//!
//! Nothing here indexes a slice and nothing divides without saying
//! what happens when it cannot, and that is not a style. Either would
//! ask for a check that can fail, a check that can fail calls
//! `core`'s panic path, and that path drags in `core::fmt`, which is
//! larger than the four kilobytes of instruction memory. `elf2vreteno`
//! refuses such an image and says so, which is how this was found.
//!
//! The addresses below are the agreement with the rest of the system:
//! `soc::DL_BASE` and `soc::DL_CTRL` are the same numbers, and the
//! word layout is the one `razboj::dl` states. The count goes in last,
//! because writing it is what tells the rasteriser the list is ready.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::write_volatile;

/// The serial port, as `hello.rs` has it.
const UART: *mut u32 = 0x3000 as *mut u32;
/// Where the display list goes, and where its count goes. These are
/// `soc::DL_BASE` and `soc::DL_CTRL`.
const DL: *mut u32 = 0x2000 as *mut u32;
const CTRL: *mut u32 = 0x4000 as *mut u32;
/// Words an instruction takes, and the words of it that say
/// anything: `razboj::dl::WORDS` and `razboj::dl::USED`.
const WORDS: usize = 8;
const USED: usize = 6;

/// The screen, which is `soc::LOGW` and `soc::H`.
const W: i32 = 128;
const H: i32 = 96;

/// The camera: the middle of the screen, the focal length in pixels,
/// and how far in front of it the solid sits.
const CX: i32 = W / 2;
const CY: i32 = H / 2;
const FOCAL: i32 = 160;
const DIST: i32 = 500;

/// The view, as a rotation matrix in eight fractional bits: the solid
/// turned twenty degrees and tipped eighteen towards the camera, so
/// that no face is square on and no edge runs straight across the
/// screen. It is worked out once, here, rather than on the core.
const M: [[i32; 3]; 3] = [[241, 0, 88], [-27, 243, 74], [-83, -79, 229]];

/// The light, in view space: from over the camera's left shoulder and
/// above. It is not a unit vector and does not need to be; the
/// brightness divides by the length of the normal, and the gain below
/// carries the rest.
const LX: i32 = -5;
const LY: i32 = 8;
const LZ: i32 = -7;
/// How far along the ramp a face is with no light on it, and how far
/// the light moves it, both in 256ths.
const AMBIENT: i32 = 40;
const GAIN: i32 = 24;
/// The colour of a face in the dark and in full light. A face's
/// colour is between the two, as far along as the light puts it.
const DARK: u32 = 0x0c_1c_54;
const LIGHT: u32 = 0x0c_dc_ff;
/// The space the solid hangs in.
const SPACE: u32 = 0x06_08_18;

/// The solid. Twelve vertices, the three golden rectangles' corners:
/// one and the golden ratio, as 62 and 100.
const NV: usize = 12;
static VX: [i16; NV] = [-62, 62, -62, 62, 0, 0, 0, 0, 100, 100, -100, -100];
static VY: [i16; NV] = [100, 100, -100, -100, -62, 62, -62, 62, 0, 0, 0, 0];
static VZ: [i16; NV] = [0, 0, 0, 0, 100, 100, -100, -100, -62, 62, -62, 62];

/// The twenty faces. The winding is not stated here: which way a face
/// points is worked out from where it sits, so a face listed either
/// way round comes out the same.
const NF: usize = 20;
static FA: [u8; NF] =
    [0, 0, 0, 0, 0, 1, 5, 11, 10, 7, 3, 3, 3, 3, 3, 4, 2, 6, 8, 9];
static FB: [u8; NF] = [
    11, 5, 1, 7, 10, 5, 11, 10, 7, 1, 9, 4, 2, 6, 8, 9, 4, 2, 6, 8,
];
static FC: [u8; NF] = [
    5, 1, 7, 10, 11, 9, 4, 2, 6, 8, 4, 2, 6, 8, 9, 5, 11, 10, 7, 1,
];

/// Send one byte, once the port is free to take it.
fn put(byte: u8) {
    unsafe {
        while UART.add(vreteno_regs::uart::STATUS / 4).read_volatile()
            & vreteno_regs::uart::STATUS_BUSY_MASK
            != 0
        {}
        write_volatile(UART.add(vreteno_regs::uart::TX / 4), byte as u32);
    }
}

fn say(line: &[u8]) {
    for byte in line {
        put(*byte);
    }
}

/// One instruction of the display list, written where the rasteriser
/// will look for it. Only the words that say something are written;
/// the rest of the eight an instruction takes are never read.
fn emit(at: usize, words: &[u32; USED]) {
    unsafe {
        for (i, w) in words.iter().enumerate() {
            write_volatile(DL.add(at * WORDS + i), *w);
        }
    }
}

/// A vertex as the display list carries it: twelve bits of two's
/// complement, in the low or the high half of a word.
fn pair(x: i32, y: i32) -> u32 {
    ((x as u32) & 0xfff) | (((y as u32) & 0xfff) << 16)
}

/// The three vertices a face is made of.
///
/// # Safety
/// `f` is one of the [`NF`] faces.
unsafe fn corners(f: usize) -> (usize, usize, usize) {
    (
        *FA.get_unchecked(f) as usize,
        *FB.get_unchecked(f) as usize,
        *FC.get_unchecked(f) as usize,
    )
}

/// A quotient, with the two checks a division asks for answered here
/// so that the compiler need not ask them: a divide by zero, which
/// cannot happen, and the one quotient that overflows, the least
/// number over minus one, which wraps as the core's `div` does. See
/// the note at the top.
fn divide(a: i32, b: i32) -> i32 {
    if b == 0 {
        return 0;
    }
    a.wrapping_div(b)
}

/// About the length of a vector, without a square root: the largest
/// part, half the next and a quarter of the least. It is within a few
/// parts in a hundred of the true length, which is closer than the
/// eye can tell one shade from the next.
fn about(x: i32, y: i32, z: i32) -> i32 {
    let (mut a, mut b, mut c) = (x.abs(), y.abs(), z.abs());
    if a < b {
        core::mem::swap(&mut a, &mut b);
    }
    if b < c {
        core::mem::swap(&mut b, &mut c);
    }
    if a < b {
        core::mem::swap(&mut a, &mut b);
    }
    a + (b >> 1) + (c >> 2)
}

/// The colour `f` 256ths of the way from [`DARK`] to [`LIGHT`], a
/// channel at a time.
fn ramp(f: i32) -> u32 {
    let ch = |shift: u32| {
        let lo = ((DARK >> shift) & 0xff) as i32;
        let hi = ((LIGHT >> shift) & 0xff) as i32;
        ((lo + (((hi - lo) * f) >> 8)) as u32) << shift
    };
    ch(16) | ch(8) | ch(0)
}

#[no_mangle]
extern "C" fn main() -> ! {
    // The vertices once turned to face the camera, and where each of
    // them lands on the screen. On the stack, so that nothing here is
    // a slice with a length the compiler would check an index against.
    let mut rxs = [0i32; NV];
    let mut rys = [0i32; NV];
    let mut rzs = [0i32; NV];
    let mut sxs = [0i32; NV];
    let mut sys = [0i32; NV];

    // The solid, turned to face the camera and dropped onto the
    // screen. Done once a vertex rather than once a face, since each
    // vertex is shared by five faces.
    for i in 0..NV {
        let (x, y, z) = unsafe {
            (
                *VX.get_unchecked(i) as i32,
                *VY.get_unchecked(i) as i32,
                *VZ.get_unchecked(i) as i32,
            )
        };
        let rx = (M[0][0] * x + M[0][1] * y + M[0][2] * z) >> 8;
        let ry = (M[1][0] * x + M[1][1] * y + M[1][2] * z) >> 8;
        let rz = (M[2][0] * x + M[2][1] * y + M[2][2] * z) >> 8;
        unsafe {
            *rxs.get_unchecked_mut(i) = rx;
            *rys.get_unchecked_mut(i) = ry;
            *rzs.get_unchecked_mut(i) = rz;
            // The screen's y grows downwards and the model's grows
            // up, so the one is the negative of the other.
            *sxs.get_unchecked_mut(i) = CX + divide(rx * FOCAL, rz + DIST);
            *sys.get_unchecked_mut(i) = CY - divide(ry * FOCAL, rz + DIST);
        }
    }

    // The list: space first, then a triangle per face that points at
    // the camera.
    emit(0, &[SPACE << 2, 0, 0, 0, 0, 0]);
    let mut at = 1usize;
    for f in 0..NF {
        let (ia, ib, ic) = unsafe { corners(f) };
        let on = |v: &[i32; NV], i: usize| unsafe { *v.get_unchecked(i) };
        let (ax, ay, az) = (on(&rxs, ia), on(&rys, ia), on(&rzs, ia));
        let (bx, by, bz) = (on(&rxs, ib), on(&rys, ib), on(&rzs, ib));
        let (cx, cy, cz) = (on(&rxs, ic), on(&rys, ic), on(&rzs, ic));
        // The face's normal, from two of its edges.
        let (ux, uy, uz) = (bx - ax, by - ay, bz - az);
        let (vx, vy, vz) = (cx - ax, cy - ay, cz - az);
        let mut nx = uy * vz - uz * vy;
        let mut ny = uz * vx - ux * vz;
        let mut nz = ux * vy - uy * vx;
        // Three times the middle of the face, in the model and, once
        // the solid is pushed away from the camera, in the view.
        let (mx, my, mz) = (ax + bx + cx, ay + by + cy, az + bz + cz);
        // Which way is out. The solid wraps the origin, so the normal
        // points outwards when it agrees with the middle of the face;
        // this is what lets the faces above be listed either way
        // round.
        if nx * mx + ny * my + nz * mz < 0 {
            nx = -nx;
            ny = -ny;
            nz = -nz;
        }
        // A face is drawn when its outward normal has some of the
        // camera in it: the far side of the solid is thrown away.
        if nx * mx + ny * my + nz * (mz + 3 * DIST) >= 0 {
            continue;
        }
        // The light, and an ambient term so that a face turned away
        // from it is dark rather than black.
        let dot = nx * LX + ny * LY + nz * LZ;
        let len = about(nx, ny, nz);
        let mut shade =
            AMBIENT + if dot > 0 { divide(dot * GAIN, len) } else { 0 };
        if shade > 255 {
            shade = 255;
        }

        let (px, py) = (on(&sxs, ia), on(&sys, ia));
        let (mut qx, mut qy) = (on(&sxs, ib), on(&sys, ib));
        let (mut rx, mut ry) = (on(&sxs, ic), on(&sys, ic));
        // The winding the rasteriser wants: inside is where none of
        // the three edge functions is negative, and that is the
        // winding whose signed area is positive.
        if (qx - px) * (ry - py) - (qy - py) * (rx - px) < 0 {
            core::mem::swap(&mut qx, &mut rx);
            core::mem::swap(&mut qy, &mut ry);
        }
        // The box to walk, clipped to the screen. The vertices are
        // not clipped: a vertex off the screen is two's complement
        // and the edge functions carry it.
        let lo = |a: i32, b: i32, c: i32| a.min(b).min(c).max(0);
        let hi = |a: i32, b: i32, c: i32, e: i32| a.max(b).max(c).min(e);
        let x0 = lo(px, qx, rx);
        let y0 = lo(py, qy, ry);
        let x1 = hi(px, qx, rx, W - 1);
        let y1 = hi(py, qy, ry, H - 1);
        if x1 < x0 || y1 < y0 {
            continue;
        }
        emit(
            at,
            &[
                2 | (ramp(shade) << 2),
                (x0 as u32) | ((y0 as u32) << 16),
                (x1 as u32) | ((y1 as u32) << 16),
                pair(px, py),
                pair(qx, qy),
                pair(rx, ry),
            ],
        );
        at += 1;
    }
    // The count, last: writing it is what says the list is ready, so
    // nothing before it may be left unwritten.
    unsafe { write_volatile(CTRL, at as u32) };

    say(b"icosahedron\n");
    // A write of one to `mhalt` stops this core. It is how a program
    // says it is done; `ebreak` is a breakpoint and traps.
    unsafe { core::arch::asm!("csrwi 0x7c0, 1", options(noreturn)) }
}

/// The entry point, at address zero, which is where the core's
/// program counter starts.
#[no_mangle]
#[link_section = ".text.init"]
pub unsafe extern "C" fn _start() -> ! {
    core::arch::asm!(
        "la sp, __stack_top",
        "la t0, __bss_start",
        "la t1, __bss_end",
        "1:",
        "beq t0, t1, 2f",
        "sw zero, 0(t0)",
        "addi t0, t0, 4",
        "j 1b",
        "2:",
        "j {main}",
        main = sym main,
        options(noreturn)
    )
}

/// Nothing can be reported and nothing can unwind, so a panic stops
/// the machine the same way a finished program does.
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    unsafe { core::arch::asm!("csrwi 0x7c0, 1", options(noreturn)) }
}
