// SPDX-License-Identifier: Apache-2.0
//! Where the textured icosahedron's cycles go on the core (#1433): the
//! parts of `ico_tex_hdmi`'s frame timed one at a time with `mcycle`,
//! with no Razboj and no scanout, so that the machine's timing mode
//! (`bazel run //cpu/vreteno:machine -- --timing --image ...`) or the
//! board can run it.
//!
//! At each of the four angles the board's cycles lines fall on, every
//! 64th frame, it times:
//! * the frame's list untextured and textured, as `ico_gl::frame`
//!   builds each, the second as `ico_tex_hdmi` builds it;
//! * the texture's setup and upload alone, `ico_gl::texture` in a new
//!   context;
//! * each list binned into a tile table where `ico_hdmi` puts one, and
//!   the records written, which is the core's share of its `draw`.
//!
//! Each line says the angle's number, then those six counts.
#![no_std]
#![no_main]

mod ico_gl;
// Only the solid, its sizes and the rectangle are used here.
#[allow(dead_code)]
mod ico_list;

use core::ptr::write_volatile;
use gles::Gl;
use ico_list::{Box, Solid, SECOND, WORDS};
use razboj_tile::{bin, ENTRIES_AT, MAX_TILES, TILE_WORDS};
use vreteno_hal::{entry, halt, trap, Razboj, Uart};

entry!(main);

/// Where `ico_tex_hdmi` keeps the texture's room.
const TEX: u32 = Razboj::LIST + 0x20_0000;

/// The slots a binned frame may take past its tile table, as
/// `ico_hdmi`'s.
const ROOM: usize = 16384;

fn mcycle() -> u32 {
    let c: u32;
    unsafe { core::arch::asm!("csrr {0}, mcycle", out(reg) c) };
    c
}

/// The first `n` slots of `list` binned where `ico_hdmi` bins them, and
/// the records written, without ringing: the core's part of its draw.
fn binned(list: &[[u32; WORDS]], n: usize, sh: u32) -> usize {
    let at = Razboj::LIST as usize + ENTRIES_AT;
    // SAFETY: the list's memory is the rasteriser's, which is not here.
    let room = unsafe {
        core::slice::from_raw_parts_mut(at as *mut [u32; WORDS], ROOM)
    };
    let mut tiles = [[0u32; TILE_WORDS]; MAX_TILES];
    let Ok(b) = bin(&list[..n], ico_list::W as u32, sh, room, &mut tiles)
    else {
        Uart::say(b"ico cost bin refused\n");
        return 0;
    };
    let base = Razboj::LIST as *mut u32;
    for (t, tile) in tiles.iter().enumerate().take(b.tiles) {
        for (w, v) in tile.iter().enumerate() {
            unsafe { write_volatile(base.add(t * TILE_WORDS + w), *v) };
        }
    }
    b.tiles
}

fn say(n: u32) {
    Uart::put(b' ');
    Uart::put_decimal(n);
}

fn main() -> ! {
    trap::say_faults();
    let solid = Solid::new();
    let model = ico_gl::Model::new(&solid);
    let mut plain = [[0u32; WORDS]; ico_gl::MOST];
    let mut tex = [[0u32; WORDS]; ico_gl::MOST];
    Uart::say(
        b"ico cost: angle plain textured upload bin_plain bin_tex tiles\n",
    );
    for k in 0..4i32 {
        // The board's lines come every 64 frames, from the frame that
        // draws into the second buffer.
        let f = 64 * k;
        let (ay, ax, dy) = ((2 * f) & 255, f & 255, SECOND);
        let sh = (ico_list::H + dy) as u32;

        let t0 = mcycle();
        let (np, _) = ico_gl::frame(
            &model,
            ay,
            ax,
            dy,
            Box::SCREEN,
            true,
            None,
            &mut plain,
        );
        let t1 = mcycle();
        // SAFETY: as `ico_hdmi`'s room, DDR3 nothing else uses.
        let room = unsafe {
            core::slice::from_raw_parts_mut(TEX as *mut u32, ico_gl::TEX_ROOM)
        };
        let (nt, _) = ico_gl::frame(
            &model,
            ay,
            ax,
            dy,
            Box::SCREEN,
            true,
            Some((room, TEX)),
            &mut tex,
        );
        let t2 = mcycle();
        let room = unsafe {
            core::slice::from_raw_parts_mut(TEX as *mut u32, ico_gl::TEX_ROOM)
        };
        let mut scratch = [[0u32; WORDS]; 2];
        let mut g = Gl::new(&mut scratch, ico_list::W as u32, sh);
        ico_gl::texture(&mut g, room, TEX);
        let t3 = mcycle();
        let _ = binned(&plain, np, sh);
        let t4 = mcycle();
        let tiles = binned(&tex, nt, sh);
        let t5 = mcycle();

        Uart::say(b"ico cost");
        say(k as u32);
        say(t1.wrapping_sub(t0));
        say(t2.wrapping_sub(t1));
        say(t3.wrapping_sub(t2));
        say(t4.wrapping_sub(t3));
        say(t5.wrapping_sub(t4));
        say(tiles as u32);
        Uart::put(b'\n');
    }
    halt()
}
