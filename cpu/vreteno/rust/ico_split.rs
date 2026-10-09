// SPDX-License-Identifier: Apache-2.0
//! The textured icosahedron's core work on one hart and on two (issue
//! 1408), timed with `mcycle` with no Razboj and no scanout, so that the
//! machine's timing mode (`bazel run //cpu/vreteno:machine -- --timing
//! --image ...`) or the board can run it.
//!
//! A frame's core work is its list, built through `ico_gl`, and the
//! list binned into a tile table where `ico_hdmi` puts one, as
//! `ico_cost` times them. On one hart the two follow each other. On two,
//! hart 1 builds the lists, each into one of two buffers in the DDR3,
//! and hart 0 bins each as it is ready, so frame `f + 1`'s list is built
//! while frame `f`'s is binned. A buffer's word says it is full, with its
//! list's length plus one, or empty, with zero.
//!
//! Frame 0 uploads the texture and is not counted. Each line says the
//! frames counted, then the cycles a frame took: on one hart its list
//! and its binning, and then on two the frame and what hart 0 waited
//! for a list.
#![no_std]
#![no_main]

mod ico_gl;
// Only the solid, its sizes and the rectangle are used here.
#[allow(dead_code)]
mod ico_list;

use core::ptr::write_volatile;
use core::sync::atomic::{AtomicU32, Ordering};
use ico_list::{Box, Solid, SECOND, WORDS};
use razboj_tile::{bin, ENTRIES_AT, MAX_TILES, TILE_WORDS};
use vreteno_hal::{entry, halt, trap, Hart1, Razboj, Uart};

entry!(main);

/// Where `ico_tex_hdmi` keeps the texture's room.
const TEX: u32 = Razboj::LIST + 0x20_0000;

/// The slots a binned frame may take past its tile table, as
/// `ico_hdmi`'s.
const ROOM: usize = 16384;

/// The frames each way, the first not counted.
const FRAMES: u32 = 9;

type List = [[u32; WORDS]; ico_gl::MOST];

/// The two lists hart 1 builds, in the DDR3 where both harts see them,
/// and the word that says whether each is full.
static mut LISTS: [List; 2] = [[[0; WORDS]; ico_gl::MOST]; 2];
static FULL: [AtomicU32; 2] = [AtomicU32::new(0), AtomicU32::new(0)];

fn mcycle() -> u32 {
    let c: u32;
    unsafe { core::arch::asm!("csrr {0}, mcycle", out(reg) c) };
    c
}

/// Frame `f`'s angles and the rows it is drawn from, as `ico_hdmi`
/// turns the solid.
fn angles(f: u32) -> (i32, i32, i32) {
    let f = f as i32;
    ((2 * f) & 255, f & 255, (f & 1) * SECOND)
}

/// Frame `f`'s list, textured, the texture uploaded on frame 0.
fn list(model: &ico_gl::Model, f: u32, out: &mut List) -> usize {
    let (ay, ax, dy) = angles(f);
    // SAFETY: as `ico_hdmi`'s room, DDR3 nothing else uses.
    let room = unsafe {
        core::slice::from_raw_parts_mut(TEX as *mut u32, ico_gl::TEX_ROOM)
    };
    let tex = Some((room, TEX, f == 0));
    ico_gl::frame(model, ay, ax, dy, Box::SCREEN, true, tex, out).0
}

/// The first `n` slots of `list` binned into the half of the room
/// `ico_hdmi` bins frame `f` into, and the records written, without
/// ringing: the core's part of its draw.
fn binned(list: &List, n: usize, f: u32) {
    let sh = (ico_list::H + angles(f).2) as u32;
    let half = ROOM / 2;
    let at = Razboj::LIST as usize
        + ENTRIES_AT
        + (f as usize & 1) * half * WORDS * 4;
    // SAFETY: the list's memory is the rasteriser's, which is not here.
    let room = unsafe {
        core::slice::from_raw_parts_mut(at as *mut [u32; WORDS], half)
    };
    let mut tiles = [[0u32; TILE_WORDS]; MAX_TILES];
    let Ok(b) = bin(&list[..n], ico_list::W as u32, sh, room, &mut tiles)
    else {
        Uart::say(b"ico split bin refused\n");
        return;
    };
    let base = Razboj::LIST as *mut u32;
    for (t, tile) in tiles.iter().enumerate().take(b.tiles) {
        for (w, v) in tile.iter().enumerate() {
            unsafe { write_volatile(base.add(t * TILE_WORDS + w), *v) };
        }
    }
}

/// Hart 1: every `step`th frame's list from frame `step - 1`, each into
/// the next of the two buffers once hart 0 has emptied it, then back to
/// wait. Bit 8 of the argument says it bins each list itself, and the
/// buffer's word then says only that the frame is done.
extern "C" fn builder(_hart: u32, arg: u32) -> ! {
    let (step, whole) = (arg & 255, arg & 256 != 0);
    let solid = Solid::new();
    let model = ico_gl::Model::new(&solid);
    let mut local = [[0u32; WORDS]; ico_gl::MOST];
    let mut f = step - 1;
    while f < FRAMES {
        let s = (f / step & 1) as usize;
        while FULL[s].load(Ordering::Acquire) != 0 {}
        // A whole frame stays in this hart's own memory.
        let n = if whole {
            let n = list(&model, f, &mut local);
            binned(&local, n, f);
            n
        } else {
            // SAFETY: hart 0 reads this buffer only while its word is
            // full.
            let out = unsafe { &mut *core::ptr::addr_of_mut!(LISTS[s]) };
            list(&model, f, out)
        };
        FULL[s].store(n as u32 + 1, Ordering::Release);
        f += step;
    }
    Hart1::park()
}

fn say(n: u32) {
    Uart::put(b' ');
    Uart::put_decimal(n);
}

/// The frames on two harts, hart 1 building every `step`th list and hart
/// 0 the rest and binning every one, from the buffer or, with `copy`,
/// from its own memory once copied there; with `whole`, hart 1 bins its
/// own. Says the cycles of the frames counted and what hart 0 waited.
fn two(
    model: &ico_gl::Model,
    step: u32,
    copy: bool,
    whole: bool,
) -> (u32, u32) {
    let mut local = [[0u32; WORDS]; ico_gl::MOST];
    Hart1::start(builder, step | (whole as u32) << 8);
    let (mut start, mut waited) = (0u32, 0u32);
    for f in 0..FRAMES {
        let w0 = mcycle();
        if f == 1 {
            start = w0;
        }
        if f % step != step - 1 {
            let n = list(model, f, &mut local);
            binned(&local, n, f);
            continue;
        }
        let s = (f / step & 1) as usize;
        let n = loop {
            let n = FULL[s].load(Ordering::Acquire);
            if n != 0 {
                break n as usize - 1;
            }
        };
        if f != 0 {
            waited += mcycle().wrapping_sub(w0);
        }
        // SAFETY: hart 1 writes this buffer only while its word is empty.
        let l = unsafe { &*core::ptr::addr_of!(LISTS[s]) };
        if whole {
            FULL[s].store(0, Ordering::Release);
        } else if copy {
            local[..n].copy_from_slice(&l[..n]);
            FULL[s].store(0, Ordering::Release);
            binned(&local, n, f);
        } else {
            binned(l, n, f);
            FULL[s].store(0, Ordering::Release);
        }
    }
    (mcycle().wrapping_sub(start), waited)
}

fn main() -> ! {
    trap::say_faults();
    let solid = Solid::new();
    let model = ico_gl::Model::new(&solid);
    let counted = FRAMES - 1;

    // One hart: the list, then its binning.
    let mut local = [[0u32; WORDS]; ico_gl::MOST];
    let (mut listed, mut bins) = (0u32, 0u32);
    for f in 0..FRAMES {
        let t0 = mcycle();
        let n = list(&model, f, &mut local);
        let t1 = mcycle();
        binned(&local, n, f);
        let t2 = mcycle();
        if f != 0 {
            listed += t1.wrapping_sub(t0);
            bins += t2.wrapping_sub(t1);
        }
    }
    Uart::say(b"ico split: frames, then cycles a frame\n");
    Uart::say(b"ico split one");
    say(counted);
    say(listed / counted);
    say(bins / counted);
    Uart::put(b'\n');
    for (name, step, copy, whole) in [
        (&b"ico split pipe"[..], 1, false, false),
        (&b"ico split pipe copied"[..], 1, true, false),
        (&b"ico split halves"[..], 2, true, false),
        (&b"ico split whole"[..], 2, false, true),
    ] {
        let (both, waited) = two(&model, step, copy, whole);
        Uart::say(name);
        say(counted);
        say(both / counted);
        say(waited / counted);
        Uart::put(b'\n');
    }
    halt()
}
