// SPDX-License-Identifier: Apache-2.0
//! The conformance suite's machine on the host (#999): EGL's machine as
//! Razboj's model draws, so that the suite's C programs draw, swap and read
//! pixels back through GL's and EGL's entry points as a program on the board
//! does.
//!
//! The framebuffer is the board's shape, 1024 words a row and 1024 rows,
//! both buffers in it. A flat list is drawn an entry at a time over what the
//! framebuffer holds, a clear over Razboj's own screen of 640 by 480; a tile
//! table is drawn whole over it, through one depth and one stencil, which no
//! two tiles share a pixel of, so that a blend reads the framebuffer as a
//! tile loaded from it does. Textures and buffer objects have rooms of their
//! own.
use gles_egl::install;
use gles_machine::Machine;
use razboj::dl::{decode, decode_list};
use razboj::model::{render_over, render_textured, Textures};
use razboj::op::Kind;
use razboj_tile::{ENTRIES_AT, TILE_WORDS, WORDS};

/// The framebuffer's words a row, and its rows.
const FW: usize = 1024;
const FH: usize = 1024;
/// Razboj's own screen, which a clear in a flat list covers.
const SW: usize = 640;
const SH: usize = 480;
/// The bus address the textures' room starts at, as Razboj reads it.
const TEX_BUS: u32 = 0x0100_0000;

struct Model {
    list: &'static mut [[u32; WORDS]],
    scratch: &'static mut [[u32; WORDS]],
    textures: &'static mut [u32],
    buffers: &'static mut [u8],
    fb: Vec<u32>,
}

impl Machine for Model {
    fn list(&mut self) -> &'static mut [[u32; WORDS]] {
        // SAFETY: the list lives as long as the program, and EGL hands it
        // to GL and to the draw in turn, never both at once.
        unsafe { &mut *(self.list as *mut [[u32; WORDS]]) }
    }

    fn draw(&mut self, entries: usize) {
        for w in &self.list[..entries] {
            let op = decode(w);
            if op.kind == Kind::Clear {
                let rows = self.fb[..FW * SH].to_vec();
                let drawn = render_over(&[op], FW, SH, rows);
                for (y, row) in drawn.chunks(FW).enumerate() {
                    self.fb[y * FW..y * FW + SW].copy_from_slice(&row[..SW]);
                }
            } else {
                let fb = std::mem::take(&mut self.fb);
                self.fb = render_over(&[op], FW, FH, fb);
            }
        }
    }

    fn scratch(&mut self) -> &'static mut [[u32; WORDS]] {
        // SAFETY: as the list's.
        unsafe { &mut *(self.scratch as *mut [[u32; WORDS]]) }
    }

    fn draw_tiled(
        &mut self,
        tiles: &[[u32; TILE_WORDS]],
        entries: &[[u32; WORDS]],
    ) {
        // The tile table laid out over the list, its records first and its
        // entries `ENTRIES_AT` past them, as the board's machine lays it
        // out where Razboj reads it (#1596): what GL keeps in its frame
        // across a draw must not rely on the list being left alone.
        let words = self.list.as_flattened_mut();
        for (k, w) in tiles.as_flattened().iter().enumerate() {
            if let Some(slot) = words.get_mut(k) {
                *slot = *w;
            }
        }
        let first = ENTRIES_AT / 4;
        for (k, w) in entries.as_flattened().iter().enumerate() {
            if let Some(slot) = words.get_mut(first + k) {
                *slot = *w;
            }
        }
        let words = &*self.textures;
        let read = |a: u32| {
            let at = (a.wrapping_sub(TEX_BUS) / 4) as usize;
            words.get(at).copied().unwrap_or(0)
        };
        let t = Textures { mem: &read };
        let ops = decode_list(entries);
        let fb = std::mem::take(&mut self.fb);
        self.fb = render_textured(&ops, FW, FH, fb, Some(&t));
    }

    fn textures(&mut self) -> Option<(&'static mut [u32], u32)> {
        // SAFETY: as the list's; the draw only reads it.
        let room = unsafe { &mut *(self.textures as *mut [u32]) };
        Some((room, TEX_BUS))
    }

    fn buffers(&mut self) -> Option<&'static mut [u8]> {
        // SAFETY: as the list's; only GL uses it.
        Some(unsafe { &mut *(self.buffers as *mut [u8]) })
    }

    fn pixels(&mut self) -> Option<(&'static [u32], usize)> {
        // SAFETY: the model lives as long as the program, and nothing draws
        // while GL reads.
        Some((unsafe { &*(self.fb.as_slice() as *const [u32]) }, FW))
    }

    fn show(&mut self, _row: u32) {}

    fn wait_blanking(&mut self) {}
}

/// Installs the model as EGL's machine, before the program's first EGL
/// call; the board's run installs the board's.
#[no_mangle]
pub extern "C" fn conform_machine() {
    let leak = |n: usize| Box::leak(vec![[0u32; WORDS]; n].into_boxed_slice());
    let m = Box::leak(Box::new(Model {
        list: leak(4096),
        scratch: leak(16384),
        textures: Box::leak(vec![0u32; 1 << 20].into_boxed_slice()),
        buffers: Box::leak(vec![0u8; 1 << 20].into_boxed_slice()),
        fb: vec![0; FW * FH],
    }));
    install(m);
}
