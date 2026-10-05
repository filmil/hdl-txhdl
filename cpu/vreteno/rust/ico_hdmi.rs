// SPDX-License-Identifier: Apache-2.0
//! A turning icosahedron on the board's HDMI output, drawn by Razboj,
//! double buffered, at 640 by 480, with the TxHDL logo in a corner
//! (issue 986).
//!
//! The core works out the solid each frame as before, and writes what
//! it sees as Razboj's display list rather than as pixels: a rectangle
//! of the backdrop over what the solid filled last time, and a
//! triangle a face. `ico_list.rs` holds that part, which has no
//! hardware in it, and a test on the host renders its lists through
//! Razboj's model.
//!
//! This program is linked for the memory at `0x4000_0000` and sent down
//! the wire by the loader, so the picture changes without Vivado.
//!
//! ## Two frames
//!
//! The scanout shows one frame while Razboj draws the other. Razboj's
//! framebuffer is fixed by its type, rows of 1024 words from
//! `0x4200_0000`, so the second frame is rows 512 to 991 of it, at
//! `0x4220_0000`, and the scanout's base moves between the two. A frame
//! goes like this:
//!
//! 1. Work out the list for the frame not shown, and write it.
//! 2. Ring the doorbell, and wait for the count to come back to zero
//!    and for the rasteriser to say it is idle: every pixel has been
//!    written.
//! 3. Point the scanout at that frame. The scanout takes its base as
//!    the vertical blanking starts, so wait for the blanking to start
//!    after the write. From then the other frame is not shown, and is
//!    the next one to draw.
//!
//! Nothing is ever drawn into the frame on the screen, so nothing
//! tears and nothing flickers, whatever the order the list is drawn in.
//!
//! ## The logo
//!
//! The core paints the logo into both frames once, at four screen
//! pixels to one of its own, in the bottom right corner. The solid
//! never reaches that corner, which the host test checks, so no clear
//! ever touches the logo and it is never drawn again.
//!
//! ## What it says
//!
//! `ico 20 faces` when it starts. Then, every 64 frames, the cycles the
//! last frame took: working out and writing the list, Razboj drawing
//! it, and the whole frame including the wait for the blanking. The
//! core-drawn version this replaced said `ico core` and the cycles its
//! drawing took, which is what these are measured against.
#![no_std]
#![no_main]

mod ico_list;

use core::ptr::{read_volatile, write_volatile};
use ico_list::{frame, rect, Box, Solid, BACKDROP, MOST, SECOND, WORDS};
use vreteno_hal::{entry, Razboj, Scan, Uart, Video};

entry!(main);

/// The logo's place: the bottom right corner, eight pixels in, four
/// screen pixels to each of its own.
const LOGO_SCALE: u32 = 4;
const LOGO_X: u32 = Scan::WIDTH - txhdl_logo::W as u32 * LOGO_SCALE - 8;
const LOGO_Y: u32 = Scan::HEIGHT - txhdl_logo::H as u32 * LOGO_SCALE - 8;

// The solid never reaches the logo's columns, so no clear erases it.
const _: () = assert!(Scan::WIDTH as i32 / 2 + ico_list::REACH < LOGO_X as i32);

/// Words from one row of the frame to the next.
const ROW: u32 = Scan::STRIDE / 4;

/// The cycle counter's low word.
fn mcycle() -> u32 {
    let c: u32;
    unsafe { core::arch::asm!("csrr {0}, mcycle", out(reg) c) };
    c
}

/// A twelve-bit colour as a scanout pixel, each four bits repeated.
fn wide(c: u32) -> u32 {
    let r = (c >> 8) & 0xf;
    let g = (c >> 4) & 0xf;
    let b = c & 0xf;
    (r * 0x11) << 16 | (g * 0x11) << 8 | b * 0x11
}

/// The first `n` entries of `list`, where the rasteriser reads them,
/// then the count. The last word is read back first, so that the posted
/// stores have landed before the count says the list is there.
fn draw(list: &[[u32; WORDS]], n: usize) {
    while Razboj::count() != 0 {}
    let base = Razboj::LIST as *mut u32;
    let mut e = 0;
    while e < n {
        let mut w = 0;
        while w < WORDS {
            unsafe { write_volatile(base.add(e * WORDS + w), list[e][w]) };
            w += 1;
        }
        e += 1;
    }
    let _ = unsafe { read_volatile(base.add(n * WORDS - 1)) };
    Razboj::ring(n as u32);
    while Razboj::count() != 0 || !Razboj::idle() {}
}

/// Paint the logo into the frame `dy` rows down. A transparent pixel is
/// left as the backdrop.
fn logo(dy: u32) {
    let base = Razboj::FRAME as *mut u32;
    let size = txhdl_logo::W as u32 * LOGO_SCALE;
    let mut r = 0u32;
    while r < size {
        let mut c = 0u32;
        while c < size {
            let word = txhdl_logo::PIXELS[((r / LOGO_SCALE)
                * txhdl_logo::W as u32
                + c / LOGO_SCALE)
                as usize];
            if word != txhdl_logo::TRANSPARENT {
                let at = ((dy + LOGO_Y + r) * ROW + LOGO_X + c) as usize;
                let px = wide(word as u32);
                unsafe { write_volatile(base.add(at), px) };
            }
            c += 1;
        }
        r += 1;
    }
}

/// Wait for the raster to leave the vertical blanking, if it is in one,
/// and then to enter the next: the moment the scanout takes its base.
fn wait_blanking() {
    while Video::blanking() {}
    while !Video::blanking() {}
}

fn main() -> ! {
    let solid = Solid::new();
    Uart::say(b"ico ");
    Uart::put_decimal(solid.found as u32);
    Uart::say(b" faces\n");

    // Both frames cleared whole, once, and the logo painted into each.
    draw(
        &[rect(BACKDROP, Box::SCREEN, 0), rect(BACKDROP, Box::SCREEN, SECOND)],
        2,
    );
    logo(0);
    logo(SECOND as u32);
    Scan::base(Razboj::FRAME);
    Scan::show(true);

    // What the solid filled last time in each frame: nothing yet, so
    // the first clear is the corner pixel, which is the backdrop
    // already. A clear of the whole screen would take the logo with it.
    const CORNER: Box = Box {
        x0: 0,
        y0: 0,
        x1: 0,
        y1: 0,
    };
    let mut last = [CORNER, CORNER];
    let mut list = [[0u32; WORDS]; MOST];
    let (mut ay, mut ax) = (0i32, 0i32);
    let mut frames = 0u32;
    let mut which = 1usize;
    loop {
        let start = mcycle();
        let dy = which as i32 * SECOND;
        let (n, filled) = frame(&solid, ay, ax, dy, last[which], &mut list);
        last[which] = filled;
        let listed = mcycle();
        draw(&list, n);
        let drawn = mcycle();
        Scan::base(Razboj::FRAME + (dy as u32) * Scan::STRIDE);
        wait_blanking();
        let shown = mcycle();

        if frames & 63 == 0 {
            Uart::say(b"ico razboj list ");
            Uart::put_decimal(listed.wrapping_sub(start));
            Uart::say(b" draw ");
            Uart::put_decimal(drawn.wrapping_sub(listed));
            Uart::say(b" frame ");
            Uart::put_decimal(shown.wrapping_sub(start));
            Uart::put(b'\n');
        }
        frames = frames.wrapping_add(1);
        which ^= 1;
        ay = (ay + 2) & 255;
        ax = (ax + 1) & 255;
    }
}
