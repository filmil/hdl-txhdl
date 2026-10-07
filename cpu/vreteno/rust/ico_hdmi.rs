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
//! The core paints the logo into both frames once, a pixel of its own
//! to a pixel of the screen, in the bottom right corner (issue 1213). The solid
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
//!
//! ## Through GL
//!
//! Built with `--cfg=gl`, as `ico_gl_hdmi`, the list comes from
//! `ico_gl.rs` instead, the same frames written through the GL ES
//! library of `//gles` (issue 995), and the cycles line says `ico gl
//! list` where this one says `ico razboj list`. Everything else is the
//! same program, so the two lines measure what the library costs.
//!
//! Through GL the back faces are hidden by the depth test rather than
//! culled (#1273): every face is drawn, and each pixel keeps the
//! nearest. Razboj tests depth only in a tile table, so the frame is
//! binned into one at the list and rung as one, and the draw line then
//! counts the binning and the tile buffer's write-out too.
#![no_std]
#![no_main]

#[cfg(gl)]
mod ico_gl;
// Through GL the hand-written list is not drawn, only its solid and
// its rectangle are used.
#[cfg_attr(gl, allow(dead_code))]
mod ico_list;

use core::ptr::{read_volatile, write_volatile};
#[cfg(not(gl))]
use ico_list::MOST;
use ico_list::{rect, Box, Solid, BACKDROP, SECOND, WORDS};
use vreteno_hal::{entry, trap, Razboj, Scan, Uart, Video};

entry!(main);

/// The logo's place: the bottom right corner, eight pixels in, a pixel
/// of its own to a pixel of the screen (issue 1213).
const LOGO_X: u32 = Scan::WIDTH - txhdl_logo::W as u32 - 8;
const LOGO_Y: u32 = Scan::HEIGHT - txhdl_logo::H as u32 - 8;

// The solid never reaches the logo's columns, so no clear erases it.
const _: () = assert!(Scan::WIDTH as i32 / 2 + ico_list::REACH < LOGO_X as i32);

/// What the cycles line starts with: the list written by hand, or
/// through the GL ES library (issue 995).
#[cfg(not(gl))]
const SAYS: &[u8] = b"ico razboj list ";
#[cfg(gl)]
const SAYS: &[u8] = b"ico gl list ";

/// Words from one row of the frame to the next.
const ROW: u32 = Scan::STRIDE / 4;

/// The cycle counter's low word.
fn mcycle() -> u32 {
    let c: u32;
    unsafe { core::arch::asm!("csrr {0}, mcycle", out(reg) c) };
    c
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

/// The slots of entries a binned frame may take at the list, past its
/// tile table: a megabyte of the list's four.
#[cfg(gl)]
const ROOM: usize = 16384;

/// The first `n` slots of `list`, a frame that tests depth on a screen
/// `sh` rows high, binned into a tile table where the rasteriser reads
/// one (#1273): the entries straight to `ENTRIES_AT` past the list, the
/// records at it, then the count with the bit that says it is a tile
/// table. Razboj tests depth only in a tile table.
#[cfg(gl)]
fn draw_tiled(list: &[[u32; WORDS]], n: usize, sh: u32) {
    use razboj_tile::{bin, ENTRIES_AT, MAX_TILES, TILED, TILE_WORDS};
    while Razboj::count() != 0 {}
    let at = Razboj::LIST as usize + ENTRIES_AT;
    // SAFETY: the list's memory is Razboj's, and it reads none of it
    // until it is rung.
    let room = unsafe {
        core::slice::from_raw_parts_mut(at as *mut [u32; WORDS], ROOM)
    };
    let mut tiles = [[0u32; TILE_WORDS]; MAX_TILES];
    let Ok(b) = bin(&list[..n], ico_list::W as u32, sh, room, &mut tiles)
    else {
        Uart::say(b"ico bin refused\n");
        return;
    };
    let base = Razboj::LIST as *mut u32;
    let mut t = 0;
    while t < b.tiles {
        let mut w = 0;
        while w < TILE_WORDS {
            let v = tiles[t][w];
            unsafe { write_volatile(base.add(t * TILE_WORDS + w), v) };
            w += 1;
        }
        t += 1;
    }
    let last = (at + b.entries * WORDS * 4 - 4) as *const u32;
    let _ = unsafe { read_volatile(last) };
    Razboj::ring(b.tiles as u32 | TILED);
    while Razboj::count() != 0 || !Razboj::idle() {}
}

/// Paint the logo into the frame `dy` rows down. A transparent pixel is
/// left as the backdrop.
fn logo(dy: u32) {
    let base = Razboj::FRAME as *mut u32;
    let mut r = 0usize;
    while r < txhdl_logo::H {
        let mut c = 0usize;
        while c < txhdl_logo::W {
            if let Some(px) = txhdl_logo::colour(c, r) {
                let row = dy + LOGO_Y + r as u32;
                let at = (row * ROW + LOGO_X + c as u32) as usize;
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
    // A fault says where it was and what it read, rather than leaving
    // the loader only the cause and the instruction (#1214).
    trap::say_faults();
    let solid = Solid::new();
    Uart::say(b"ico ");
    Uart::put_decimal(solid.found as u32);
    Uart::say(b" faces\n");

    // Both frames cleared whole, once, and the logo painted into each.
    draw(
        &[
            rect(BACKDROP, Box::SCREEN, 0),
            rect(BACKDROP, Box::SCREEN, SECOND),
        ],
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
    #[cfg(not(gl))]
    let mut list = [[0u32; WORDS]; MOST];
    #[cfg(gl)]
    let mut list = [[0u32; WORDS]; ico_gl::MOST];
    #[cfg(gl)]
    let model = ico_gl::Model::new(&solid);
    let (mut ay, mut ax) = (0i32, 0i32);
    let mut frames = 0u32;
    let mut which = 1usize;
    loop {
        let start = mcycle();
        let dy = which as i32 * SECOND;
        #[cfg(not(gl))]
        let (n, filled) =
            ico_list::frame(&solid, ay, ax, dy, last[which], &mut list);
        #[cfg(gl)]
        let (n, filled) =
            ico_gl::frame(&model, ay, ax, dy, last[which], true, &mut list);
        last[which] = filled;
        let listed = mcycle();
        #[cfg(not(gl))]
        draw(&list, n);
        #[cfg(gl)]
        draw_tiled(&list, n, (ico_list::H + dy) as u32);
        let drawn = mcycle();
        Scan::base(Razboj::FRAME + (dy as u32) * Scan::STRIDE);
        wait_blanking();
        let shown = mcycle();

        if frames & 63 == 0 {
            Uart::say(SAYS);
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
