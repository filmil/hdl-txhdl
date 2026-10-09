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
//!
//! ## Two harts
//!
//! Through GL the second hart, where the bitstream has one, builds and
//! bins the odd frames while this one does the even ones (#1408): each
//! hart draws into its own frame and bins into its own half of the
//! room, so the two share only the odd frame's records and three words.
//! Hart 0 still rings every frame, waits for every drawing and moves the
//! scanout, so the frames are shown in the order they were before. On
//! the machine, `ico_split` measured the core's part of a textured frame
//! at half what one hart takes.
#![no_std]
#![no_main]

#[cfg(gl)]
// Untextured, the texture's room is not used.
#[cfg_attr(not(tex), allow(dead_code))]
mod ico_gl;
// Through GL the hand-written list is not drawn, only its solid and
// its rectangle are used.
#[cfg_attr(gl, allow(dead_code))]
mod ico_list;

use core::ptr::{read_volatile, write_volatile};
#[cfg(gl)]
use core::sync::atomic::{AtomicU32, Ordering};
#[cfg(not(gl))]
use ico_list::MOST;
use ico_list::{rect, Box, Solid, BACKDROP, SECOND, WORDS};
#[cfg(gl)]
use vreteno_hal::Hart1;
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
#[cfg(all(gl, not(tex)))]
const SAYS: &[u8] = b"ico gl list ";
#[cfg(all(tex, not(mip)))]
const SAYS: &[u8] = b"ico gl tex list ";
#[cfg(mip)]
const SAYS: &[u8] = b"ico gl mip list ";

/// Where the texture's room is with `tex` (#997): in the list's memory,
/// two megabytes past its start and past the tile table's megabyte.
/// Razboj reads it at the address the core writes it at.
#[cfg(tex)]
const TEX: u32 = Razboj::LIST + 0x20_0000;

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
/// tile table: a megabyte of the list's four, in two halves, one for the
/// frame Razboj draws and one for the frame the core builds (#1433).
#[cfg(gl)]
const ROOM: usize = 16384;
#[cfg(gl)]
const HALF: usize = ROOM / 2;

/// A frame binned and not yet rung: its tiles' records and how many.
#[cfg(gl)]
struct Binned {
    tiles: [[u32; razboj_tile::TILE_WORDS]; razboj_tile::MAX_TILES],
    count: usize,
    last: usize,
}

/// The first `n` slots of `list`, a frame that tests depth on a screen
/// `sh` rows high, binned into the half `half` of the entries' room past
/// the list (#1273), and its records kept to be written when the frame
/// before is drawn: each record's first entry is counted from the room's
/// start, so it is moved by the half's place. Razboj reads nothing of
/// this half while it draws from the other.
#[cfg(gl)]
fn bin_tiled(list: &[[u32; WORDS]], n: usize, sh: u32, half: usize) -> Binned {
    use razboj_tile::{bin, ENTRIES_AT, MAX_TILES, TILE_WORDS};
    let off = half * HALF;
    let at = Razboj::LIST as usize + ENTRIES_AT + off * WORDS * 4;
    // SAFETY: the list's memory is Razboj's, and it reads none of this
    // half until it is rung with it.
    let room = unsafe {
        core::slice::from_raw_parts_mut(at as *mut [u32; WORDS], HALF)
    };
    let mut b = Binned {
        tiles: [[0u32; TILE_WORDS]; MAX_TILES],
        count: 0,
        last: at,
    };
    let Ok(r) = bin(&list[..n], ico_list::W as u32, sh, room, &mut b.tiles)
    else {
        Uart::say(b"ico bin refused\n");
        return b;
    };
    let mut t = 0;
    while t < r.tiles {
        b.tiles[t][0] += off as u32;
        t += 1;
    }
    b.count = r.tiles;
    b.last = at + r.entries * WORDS * 4 - 4;
    b
}

/// A binned frame's records at the list, then the count with the bit
/// that says it is a tile table. The last entry is read back first, so
/// that the posted stores have landed before the count says it is there.
/// It does not wait for the drawing.
#[cfg(gl)]
fn ring_tiled(b: &Binned) {
    use razboj_tile::{TILED, TILE_WORDS};
    let base = Razboj::LIST as *mut u32;
    let mut t = 0;
    while t < b.count {
        let mut w = 0;
        while w < TILE_WORDS {
            let v = b.tiles[t][w];
            unsafe { write_volatile(base.add(t * TILE_WORDS + w), v) };
            w += 1;
        }
        t += 1;
    }
    let _ = unsafe { read_volatile(b.last as *const u32) };
    Razboj::ring(b.count as u32 | TILED);
}

/// Wait for Razboj to have drawn what it was rung with.
#[cfg(gl)]
fn wait_drawn() {
    while Razboj::count() != 0 || !Razboj::idle() {}
}

/// Where a frame starts to clear, before the solid has filled anything
/// in its buffer: the corner pixel, which is the backdrop already. A
/// clear of the whole screen would take the logo with it.
const CORNER: Box = Box {
    x0: 0,
    y0: 0,
    x1: 0,
    y1: 0,
};

/// Hart 1 has started (#1408).
#[cfg(gl)]
static ALIVE: AtomicU32 = AtomicU32::new(0);

/// Every frame before this one is drawn: the half of the room the frame
/// before last was binned into is free.
#[cfg(gl)]
static DRAWN: AtomicU32 = AtomicU32::new(0);

/// An odd frame binned by hart 1 and not yet rung, and its number plus
/// one while it waits, zero once hart 0 has rung it.
#[cfg(gl)]
static mut ODD: Binned = Binned {
    tiles: [[0; razboj_tile::TILE_WORDS]; razboj_tile::MAX_TILES],
    count: 0,
    last: 0,
};
#[cfg(gl)]
static ODD_READY: AtomicU32 = AtomicU32::new(0);

/// The texture `ico_gl::frame` is given: none untextured, else the
/// room, the checker uploaded into it when `upload`.
#[cfg(gl)]
fn texture(upload: bool) -> Option<(&'static mut [u32], u32, bool)> {
    #[cfg(not(tex))]
    {
        let _ = upload;
        None
    }
    // SAFETY: the room is DDR3 nothing else uses. Razboj reads it while
    // it draws, and after the first frame nothing writes it.
    #[cfg(tex)]
    {
        let room = unsafe {
            core::slice::from_raw_parts_mut(TEX as *mut u32, ico_gl::TEX_ROOM)
        };
        Some((room, TEX, upload))
    }
}

/// Hart 1 (#1408): the odd frames, each built into the first buffer, at
/// row 0, and binned
/// into the second half of the room once the frame before last, which
/// took that half, is drawn, then left for hart 0 to ring. Its entries
/// are read back before it says they are there, so that they have
/// landed when Razboj reads them.
#[cfg(gl)]
extern "C" fn odd_frames(_hart: u32, _arg: u32) -> ! {
    ALIVE.store(1, Ordering::Release);
    let solid = Solid::new();
    let model = ico_gl::Model::new(&solid);
    let mut list = [[0u32; WORDS]; ico_gl::MOST];
    let mut last = CORNER;
    let mut f = 1u32;
    loop {
        let (ay, ax) = ((2 * f as i32) & 255, f as i32 & 255);
        let tex = texture(false);
        // The odd frames go to the first buffer: hart 0 starts with
        // `which` at 1, so its even frames go to the second, and the
        // scanout shows the buffer the frame before was drawn into. Both
        // harts drew into the second, and the first, with the logo and
        // nothing else, was shown every other frame (#1551).
        let (n, filled) =
            ico_gl::frame(&model, ay, ax, SECOND, last, true, tex, &mut list);
        last = filled;
        while DRAWN.load(Ordering::Acquire) < f - 1 {}
        let sh = (ico_list::H + SECOND) as u32;
        let b = bin_tiled(&list, n, sh, 1);
        let _ = unsafe { read_volatile(b.last as *const u32) };
        while ODD_READY.load(Ordering::Acquire) != 0 {}
        // SAFETY: hart 0 reads the records only while they are ready.
        unsafe { core::ptr::addr_of_mut!(ODD).write(b) };
        ODD_READY.store(f + 1, Ordering::Release);
        f = f.wrapping_add(2);
    }
}

/// Start hart 1 on the odd frames, and say whether it answered within
/// ten milliseconds; a bitstream with one hart never does.
#[cfg(gl)]
fn start_odd() -> bool {
    Hart1::start(odd_frames, 0);
    let t = mcycle();
    while mcycle().wrapping_sub(t) < 1_000_000 {
        if ALIVE.load(Ordering::Acquire) != 0 {
            return true;
        }
    }
    false
}

/// What a load that draws nothing is doing (#1551), said after the
/// cycles: the tiles rung for this frame, the doorbell's count and
/// whether Razboj was idle straight after the ring, its status word,
/// the buffer just shown and the base the scanout reads back, and for
/// each buffer what [`drawn_in`] found.
#[cfg(diag)]
fn diag(
    (tiles, count, idle): (u32, u32, bool),
    shown: usize,
    seen: [(u32, u32); 2],
) {
    Uart::say(b" tiles ");
    Uart::put_decimal(tiles);
    Uart::say(b" rung ");
    Uart::put_decimal(count);
    Uart::say(if idle { b" idle" } else { b" busy" });
    Uart::say(b" shows ");
    Uart::put_decimal(shown as u32);
    Uart::say(b" base ");
    Uart::put_hex(Scan::base_word());
    Uart::say(b" status ");
    Uart::put_hex(Razboj::status_word());
    for (b, (drawn, centre)) in seen.iter().enumerate() {
        Uart::say(if b == 0 { b" drawn0 " } else { b" drawn1 " });
        Uart::put_decimal(*drawn);
        Uart::say(b" centre ");
        Uart::put_hex(*centre);
    }
}

/// How many pixels of buffer `b`'s middle row are not the backdrop, and
/// its centre pixel as it reads, each read with `lr.w`, which goes
/// around the data cache (#1551); a plain load may be answered from it.
/// A run of them holds the other hosts' writes at the arbiter (#1408),
/// so this is called while Razboj is idle. The program is built without
/// the A extension, so the instruction is given by its encoding.
#[cfg(diag)]
fn drawn_in(b: usize) -> (u32, u32) {
    let row = b as u32 * SECOND as u32 + ico_list::H as u32 / 2;
    let at = Razboj::FRAME + row * Scan::STRIDE;
    let lr = |a: u32| -> u32 {
        let px: u32;
        // SAFETY: a load of the buffer's row; `lr.w` is
        // `.insn r 0x2f, 2, 8, rd, rs1, x0`.
        unsafe {
            core::arch::asm!(
                ".insn r 0x2f, 2, 8, {0}, {1}, x0",
                out(reg) px,
                in(reg) a,
            )
        };
        px
    };
    let mut drawn = 0u32;
    let mut x = 0u32;
    while x < ico_list::W as u32 {
        if lr(at + 4 * x) & 0x00ff_ffff != BACKDROP {
            drawn += 1;
        }
        x += 1;
    }
    (drawn, lr(at + 4 * (ico_list::W as u32 / 2)))
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

    // What the solid filled last time in each frame: nothing yet.
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
    // By hand the frame is serial: its list, its drawing, its showing.
    #[cfg(not(gl))]
    loop {
        let start = mcycle();
        let dy = which as i32 * SECOND;
        let (n, filled) =
            ico_list::frame(&solid, ay, ax, dy, last[which], &mut list);
        last[which] = filled;
        let listed = mcycle();
        draw(&list, n);
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
    // Through GL a frame's list is built and binned while Razboj draws
    // the frame before (#1433). Then the core waits for that drawing,
    // shows it from the next blanking, which frees the buffer this frame
    // draws into, and rings this one. Once the first frame is built,
    // hart 1 is started on the odd frames, and this hart builds the even
    // ones and rings them all in order (#1408). The cycles line says
    // what the list and its binning took, or the wait for hart 1's; what
    // was left of the drawing to wait for; the whole frame, blanking
    // included; and the two frames up to this one's showing.
    #[cfg(gl)]
    let mut two = false;
    #[cfg(gl)]
    let mut shown_at = [0u32; 2];
    #[cfg(gl)]
    loop {
        let start = mcycle();
        let dy = which as i32 * SECOND;
        let odd = two && frames & 1 == 1;
        let mine;
        let binned = if odd {
            while ODD_READY.load(Ordering::Acquire) != frames + 1 {}
            // SAFETY: hart 1 leaves the records alone until they are rung.
            unsafe { &*core::ptr::addr_of!(ODD) }
        } else {
            // The first frame uploads the texture, and every frame after
            // it finds it where it is (#1433).
            let tex = texture(frames == 0);
            let (n, filled) = ico_gl::frame(
                &model,
                ay,
                ax,
                dy,
                last[which],
                true,
                tex,
                &mut list,
            );
            last[which] = filled;
            let sh = (ico_list::H + dy) as u32;
            mine = bin_tiled(&list, n, sh, (frames & 1) as usize);
            if frames == 0 {
                two = start_odd();
                Uart::say(if two {
                    b"ico two harts\n"
                } else {
                    b"ico one hart\n"
                });
            }
            &mine
        };
        let listed = mcycle();
        // The frame before: drawn, then shown from the next blanking.
        wait_drawn();
        DRAWN.store(frames, Ordering::Release);
        let drawn = mcycle();
        if frames != 0 {
            let before = (which ^ 1) as u32 * SECOND as u32;
            Scan::base(Razboj::FRAME + before * Scan::STRIDE);
            wait_blanking();
        }
        // Built with `diag` (#1551): the drawn pixels of the buffer just
        // shown, read while Razboj is idle, before it is rung.
        #[cfg(diag)]
        let report = frames < 4 || frames & 63 == 0;
        #[cfg(diag)]
        let seen = if report {
            [drawn_in(0), drawn_in(1)]
        } else {
            [(0, 0); 2]
        };
        ring_tiled(binned);
        // And what was rung, and what Razboj said straight after.
        #[cfg(diag)]
        let rung = (binned.count as u32, Razboj::count(), Razboj::idle());
        if odd {
            ODD_READY.store(0, Ordering::Release);
        }
        let shown = mcycle();
        let pair = shown.wrapping_sub(shown_at[which]);
        shown_at[which] = shown;
        #[cfg(not(diag))]
        let report = frames & 63 == 0;
        if report {
            Uart::say(SAYS);
            Uart::put_decimal(listed.wrapping_sub(start));
            Uart::say(b" wait ");
            Uart::put_decimal(drawn.wrapping_sub(listed));
            Uart::say(b" frame ");
            Uart::put_decimal(shown.wrapping_sub(start));
            Uart::say(b" two ");
            Uart::put_decimal(pair);
            #[cfg(diag)]
            diag(rung, which ^ 1, seen);
            Uart::put(b'\n');
        }
        frames = frames.wrapping_add(1);
        which ^= 1;
        ay = (ay + 2) & 255;
        ax = (ax + 1) & 255;
    }
}
