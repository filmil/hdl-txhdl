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
// The canonical teapot (#1592), drawn through GL in the solid's place.
#[cfg(teapot)]
mod teapot;

use core::ptr::write_volatile;
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
/// then the count, which [`Razboj::ring`] writes once the stores are
/// answered (issue 1553).
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
    b
}

/// A binned frame's records at the list, then the count with the bit
/// that says it is a tile table, which [`Razboj::ring`] writes once the
/// records' and the entries' stores are answered (issue 1553). It does
/// not wait for the drawing.
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

/// The last frame rung, so that each hart rings its own frames in turn
/// (#1639): a frame is rung once the one before it is. None at first, so
/// that hart 1's first frame waits for hart 0 to ring frame 0.
#[cfg(gl)]
static RUNG: AtomicU32 = AtomicU32::new(u32::MAX);

/// The video peripheral's count of frames shown when either hart last
/// had the scanout show a frame (#1639).
#[cfg(gl)]
static LAST_SHOW: AtomicU32 = AtomicU32::new(0);

/// A hart's showing of its frames, paced (#1639). Each hart shows the
/// frame before its own when its own is ready to ring. Shown as soon as
/// each was ready, the two harts' frames went up in pairs, back to back,
/// one of each pair on the screen for one blanking and the other for the
/// rest; so a hart first waits until half its own work has passed since
/// either hart showed a frame. Its work is the frames from its last
/// showing to its being ready again, without the wait, so that a wait
/// never lengthens the period it is worked out from. A hart's first
/// showing has no work to pace by; the waits of the next few frames put
/// the harts half a period apart, and from then on each is ready about
/// when its turn comes and waits for nothing.
#[cfg(gl)]
struct Pacer {
    /// The frames shown when this hart last showed one.
    prev: Option<u32>,
}

#[cfg(gl)]
impl Pacer {
    const fn new() -> Pacer {
        Pacer { prev: None }
    }

    /// Show the frame at `base` from the next blanking, once half this
    /// hart's work has passed since the last frame either hart showed.
    fn show(&mut self, base: u32) {
        let ready = Video::frames();
        let work = self.prev.map_or(0, |p| ready.wrapping_sub(p) & 0xffff);
        // The last showing is read before the count, so that the count is
        // never behind it; the 16-bit count's own wrap the mask takes care of.
        let since = || {
            let last = LAST_SHOW.load(Ordering::Acquire);
            Video::frames().wrapping_sub(last) & 0xffff
        };
        while since() < work / 2 {}
        Scan::base(base);
        wait_blanking();
        let now = Video::frames();
        self.prev = Some(now);
        LAST_SHOW.store(now, Ordering::Release);
    }
}

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
/// row 0, and binned into the second half of the room once the frame
/// before last, which took that half, is drawn. Once hart 0 has rung the
/// frame before, it shows that one, paced, and rings its own (#1639).
#[cfg(gl)]
extern "C" fn odd_frames(_hart: u32, _arg: u32) -> ! {
    ALIVE.store(1, Ordering::Release);
    let solid = Solid::new();
    let model = ico_gl::Model::new(&solid);
    #[cfg(not(teapot))]
    let mut list = [[0u32; WORDS]; ico_gl::MOST];
    #[cfg(teapot)]
    let mut list = lists(1);
    let mut last = CORNER;
    let mut pacer = Pacer::new();
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
            ico_gl::frame(&model, ay, ax, 0, last, true, tex, &mut list);
        last = filled;
        while DRAWN.load(Ordering::Acquire) < f - 1 {}
        let sh = ico_list::H as u32;
        let b = bin_tiled(&list[..], n, sh, 1);
        // The frame before is hart 0's, in the second buffer: drawn, then
        // shown, paced, and this one rung.
        while RUNG.load(Ordering::Acquire) != f - 1 {}
        wait_drawn();
        DRAWN.store(f, Ordering::Release);
        pacer.show(Razboj::FRAME + SECOND as u32 * Scan::STRIDE);
        ring_tiled(&b);
        RUNG.store(f, Ordering::Release);
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

/// The canonical teapot's lists (#1592), one a hart: a frame of it is
/// more than the stack's 64 KiB.
#[cfg(teapot)]
static mut LISTS: [[[u32; WORDS]; ico_gl::MOST]; 2] =
    [[[0; WORDS]; ico_gl::MOST]; 2];

/// Hart `hart`'s list, which only that hart touches.
#[cfg(teapot)]
fn lists(hart: usize) -> &'static mut [[u32; WORDS]; ico_gl::MOST] {
    // SAFETY: each hart takes its own once, and nothing else does.
    unsafe { &mut (*core::ptr::addr_of_mut!(LISTS))[hart] }
}

/// The canonical demos (#1592): the series an image is in, the
/// icosahedron's or the teapot's, its step in it and its name, given by
/// the build; an image without them runs for good, as the demos before
/// them do.
#[cfg(not(teapot))]
const SERIES: &[u8] = b"ico";
#[cfg(teapot)]
const SERIES: &[u8] = b"teapot";
const STEP: Option<&str> = option_env!("DEMO_STEP");
const NAME: Option<&str> = option_env!("DEMO_NAME");

/// How long a canonical demo runs: 1200 frames, 20 seconds at 60 Hz;
/// the teapot, 256, two turns, since a frame of it takes several of the
/// scanout's (#1592).
#[cfg(not(teapot))]
const DEMO_FRAMES: u32 = 1200;
#[cfg(teapot)]
const DEMO_FRAMES: u32 = 256;

/// Where the step's title goes: the top left corner, eight pixels in,
/// each of a 5 by 7 font's pixels two of the screen's, a glyph twelve
/// pixels apart.
const TITLE_X: u32 = 8;
const TITLE_Y: u32 = 8;
const TITLE_H: u32 = 14;

// The solid never reaches the title's rows, so no clear erases it.
const _: () =
    assert!(ico_list::H / 2 - ico_list::REACH > (TITLE_Y + TITLE_H) as i32);

/// A 5 by 7 glyph, a byte a column from the left, bit 0 at the top: the
/// digits, the small letters, a space, a colon and a dash, the classic
/// LCD font's; anything else is a space.
fn glyph(c: u8) -> [u8; 5] {
    const DIGITS: [[u8; 5]; 10] = [
        [0x3e, 0x51, 0x49, 0x45, 0x3e],
        [0x00, 0x42, 0x7f, 0x40, 0x00],
        [0x42, 0x61, 0x51, 0x49, 0x46],
        [0x21, 0x41, 0x45, 0x4b, 0x31],
        [0x18, 0x14, 0x12, 0x7f, 0x10],
        [0x27, 0x45, 0x45, 0x45, 0x39],
        [0x3c, 0x4a, 0x49, 0x49, 0x30],
        [0x01, 0x71, 0x09, 0x05, 0x03],
        [0x36, 0x49, 0x49, 0x49, 0x36],
        [0x06, 0x49, 0x49, 0x29, 0x1e],
    ];
    const LETTERS: [[u8; 5]; 26] = [
        [0x20, 0x54, 0x54, 0x54, 0x78],
        [0x7f, 0x48, 0x44, 0x44, 0x38],
        [0x38, 0x44, 0x44, 0x44, 0x20],
        [0x38, 0x44, 0x44, 0x48, 0x7f],
        [0x38, 0x54, 0x54, 0x54, 0x18],
        [0x08, 0x7e, 0x09, 0x01, 0x02],
        [0x0c, 0x52, 0x52, 0x52, 0x3e],
        [0x7f, 0x08, 0x04, 0x04, 0x78],
        [0x00, 0x44, 0x7d, 0x40, 0x00],
        [0x20, 0x40, 0x44, 0x3d, 0x00],
        [0x7f, 0x10, 0x28, 0x44, 0x00],
        [0x00, 0x41, 0x7f, 0x40, 0x00],
        [0x7c, 0x04, 0x18, 0x04, 0x78],
        [0x7c, 0x08, 0x04, 0x04, 0x78],
        [0x38, 0x44, 0x44, 0x44, 0x38],
        [0x7c, 0x14, 0x14, 0x14, 0x08],
        [0x08, 0x14, 0x14, 0x18, 0x7c],
        [0x7c, 0x08, 0x04, 0x04, 0x08],
        [0x48, 0x54, 0x54, 0x54, 0x20],
        [0x04, 0x3f, 0x44, 0x40, 0x20],
        [0x3c, 0x40, 0x40, 0x20, 0x7c],
        [0x1c, 0x20, 0x40, 0x20, 0x1c],
        [0x3c, 0x40, 0x30, 0x40, 0x3c],
        [0x44, 0x28, 0x10, 0x28, 0x44],
        [0x0c, 0x50, 0x50, 0x50, 0x3c],
        [0x44, 0x64, 0x54, 0x4c, 0x44],
    ];
    match c {
        b'0'..=b'9' => DIGITS[(c - b'0') as usize],
        b'a'..=b'z' => LETTERS[(c - b'a') as usize],
        b':' => [0x00, 0x36, 0x36, 0x00, 0x00],
        b'-' => [0x08, 0x08, 0x08, 0x08, 0x08],
        _ => [0; 5],
    }
}

/// Paint `text` in white into the frame `dy` rows down at the title's
/// place, once, as the logo is: the backdrop stays between its strokes.
fn title(text: &[u8], dy: u32) {
    let base = Razboj::FRAME as *mut u32;
    for (k, &c) in text.iter().enumerate() {
        let g = glyph(c);
        for (i, col) in g.iter().enumerate() {
            for j in 0..7u32 {
                if col >> j & 1 == 0 {
                    continue;
                }
                let x = TITLE_X + 12 * k as u32 + 2 * i as u32;
                let y = dy + TITLE_Y + 2 * j;
                for (ox, oy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let at = ((y + oy) * ROW + x + ox) as usize;
                    // SAFETY: the frame's pixels, which nothing draws over
                    // at the title's place.
                    unsafe { write_volatile(base.add(at), 0x00ff_ffff) };
                }
            }
        }
    }
}

/// A canonical demo's start (#1592): the console line naming its step,
/// and the title over both frames.
fn demo_start() {
    let (Some(step), Some(name)) = (STEP, NAME) else {
        return;
    };
    Uart::say(b"demo ");
    Uart::say(SERIES);
    Uart::put(b' ');
    Uart::say(step.as_bytes());
    Uart::say(b": ");
    Uart::say(name.as_bytes());
    Uart::put(b'\n');
    #[cfg(teapot)]
    {
        Uart::say(b"teapot ");
        Uart::put_decimal(teapot::get().triangles as u32);
        Uart::say(b" triangles\n");
    }
    START.store(Video::frames(), Ordering::Relaxed);
    let mut text = [b' '; 40];
    let mut n = 0;
    for part in [SERIES, b" ", step.as_bytes(), b": ", name.as_bytes()] {
        for &c in part {
            if n < text.len() {
                text[n] = c;
                n += 1;
            }
        }
    }
    title(&text[..n], 0);
    title(&text[..n], SECOND as u32);
}

/// The video peripheral's count of frames shown when a canonical demo
/// starts, for the rate it says at the end.
static START: AtomicU32 = AtomicU32::new(0);

/// A canonical demo's end once it has run its frames (#1592): the
/// console says so, with the frames drawn against the frames the
/// scanout showed meanwhile, as frames a second at 60 Hz in tenths, and
/// the core stops, the last frame shown. The rate is a canonical demo's
/// measure from one milestone to the next.
fn demo_end(frames: u32) {
    if let Some(step) = STEP {
        if frames >= DEMO_FRAMES {
            let shown = Video::frames()
                .wrapping_sub(START.load(Ordering::Relaxed))
                & 0xffff;
            Uart::say(b"demo ");
            Uart::say(SERIES);
            Uart::put(b' ');
            Uart::say(step.as_bytes());
            Uart::say(b" done: ");
            Uart::put_decimal(frames);
            Uart::say(b" frames in ");
            Uart::put_decimal(shown);
            Uart::say(b" shown, fps x10 ");
            Uart::put_decimal(frames * 600 / shown.max(1));
            Uart::put(b'\n');
            vreteno_hal::halt();
        }
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
    demo_start();
    Scan::base(Razboj::FRAME);
    Scan::show(true);
    // Built with `ahead`, the scanout asks for its lines two rows ahead
    // rather than one (#1523).
    #[cfg(ahead)]
    Scan::two_ahead(true);

    // What the solid filled last time in each frame: nothing yet.
    let mut last = [CORNER, CORNER];
    #[cfg(not(gl))]
    let mut list = [[0u32; WORDS]; MOST];
    #[cfg(all(gl, not(teapot)))]
    let mut list = [[0u32; WORDS]; ico_gl::MOST];
    #[cfg(teapot)]
    let mut list = lists(0);
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
        demo_end(frames);
        which ^= 1;
        ay = (ay + 2) & 255;
        ax = (ax + 1) & 255;
    }
    // Through GL a frame's list is built and binned while Razboj draws
    // the frame before (#1433). Then the core waits for that drawing,
    // shows it from the next blanking, which frees the buffer this frame
    // draws into, and rings this one. Once the first frame is built,
    // hart 1 is started on the odd frames, and this hart builds the even
    // ones (#1408); each hart shows the frame before its own, paced, and
    // rings its own in turn (#1639). The cycles line says what the list
    // and its binning took; what was left of the frame before to wait
    // for, hart 1's ringing and the drawing; the whole frame, blanking
    // included; and the two frames up to this one's showing.
    #[cfg(gl)]
    let mut two = false;
    #[cfg(gl)]
    let mut pacer = Pacer::new();
    #[cfg(gl)]
    let mut shown_at = [0u32; 2];
    #[cfg(gl)]
    loop {
        let start = mcycle();
        let dy = which as i32 * SECOND;
        let binned = {
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
            // The frame before last took this half of the room; with
            // two harts it is hart 1 that waits for its drawing.
            if two {
                while DRAWN.load(Ordering::Acquire) < frames - 1 {}
            }
            let mine = bin_tiled(&list[..], n, sh, (frames & 1) as usize);
            if frames == 0 {
                two = start_odd();
                Uart::say(if two {
                    b"ico two harts\n"
                } else {
                    b"ico one hart\n"
                });
            }
            mine
        };
        let binned = &binned;
        let listed = mcycle();
        // The frame before, hart 1's with two harts, once rung: drawn,
        // then shown from the next blanking, paced.
        if two && frames != 0 {
            while RUNG.load(Ordering::Acquire) != frames - 1 {}
        }
        wait_drawn();
        DRAWN.store(frames, Ordering::Release);
        let drawn = mcycle();
        if frames != 0 {
            let before = (which ^ 1) as u32 * SECOND as u32;
            pacer.show(Razboj::FRAME + before * Scan::STRIDE);
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
        RUNG.store(frames, Ordering::Release);
        // And what was rung, and what Razboj said straight after.
        #[cfg(diag)]
        let rung = (binned.count as u32, Razboj::count(), Razboj::idle());
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
            // How close the scanout came since the last line: its
            // longest line, in pixels, and its lines late at their rows
            // (#1523).
            let (worst, lates) = Scan::timing();
            Uart::say(b" worst ");
            Uart::put_decimal(worst);
            Uart::say(b" lates ");
            Uart::put_decimal(lates);
            // The rows and frames that showed `LATE`, and the least
            // margin, in pixels (#1524).
            let (rows, frames_late, margin) = Scan::late();
            Uart::say(b" late_rows ");
            Uart::put_decimal(rows);
            Uart::say(b" late_frames ");
            Uart::put_decimal(frames_late);
            Uart::say(b" margin ");
            if margin < 0 {
                Uart::put(b'-');
            }
            Uart::put_decimal(margin.unsigned_abs());
            Scan::clear();
            Uart::put(b'\n');
        }
        frames = frames.wrapping_add(1);
        demo_end(frames);
        which ^= 1;
        ay = (ay + 2) & 255;
        ax = (ax + 1) & 255;
        // With two harts the odd frame is hart 1's to build and ring.
        if two {
            frames = frames.wrapping_add(1);
            demo_end(frames);
            which ^= 1;
            ay = (ay + 2) & 255;
            ax = (ax + 1) & 255;
        }
    }
}
