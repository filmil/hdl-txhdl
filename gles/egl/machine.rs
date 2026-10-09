// SPDX-License-Identifier: Apache-2.0
//! What an EGL swap needs of the machine it runs on (issue 996), alone,
//! so that a machine can be written without linking EGL's entry points:
//! a program that defines `#[no_mangle]` functions keeps every one, and
//! a program in the boot memory has no room for them.
#![no_std]

use razboj_tile::{TILE_WORDS, WORDS};

/// What a swap needs of the machine.
pub trait Machine {
    /// Razboj's display list, where GL writes a frame for it to read.
    fn list(&mut self) -> &'static mut [[u32; WORDS]];
    /// Has Razboj draw the first `entries` of the list, and waits until
    /// every pixel of them is written.
    fn draw(&mut self, entries: usize);
    /// Room apart from the list, which a frame that tests depth is
    /// binned into before it is drawn as a tile table (#1273).
    fn scratch(&mut self) -> &'static mut [[u32; WORDS]];
    /// Has Razboj draw a tile table: `tiles`, the records, laid out at
    /// the list, and `entries` from `razboj_tile::ENTRIES_AT` past it,
    /// rung with the count's bit that says it is one; and waits until
    /// every pixel of it is written.
    fn draw_tiled(
        &mut self,
        tiles: &[[u32; TILE_WORDS]],
        entries: &[[u32; WORDS]],
    );
    /// Room for the textures (#997), and the bus address Razboj reads
    /// its first word at: none, the default, leaves GL without textures,
    /// and `glTexImage2D` then fails with `GL_OUT_OF_MEMORY`.
    fn textures(&mut self) -> Option<(&'static mut [u32], u32)> {
        None
    }
    /// Room for the buffer objects' stores (#1488): none, the default,
    /// leaves `glBufferData` failing with `GL_OUT_OF_MEMORY`. Only the core
    /// reads it, so it needs no bus address.
    fn buffers(&mut self) -> Option<&'static mut [u8]> {
        None
    }
    /// The framebuffer as the core reads it, for `glReadPixels` (#999): its
    /// words from Razboj's row nought, each `0xAARRGGBB`, and the words
    /// from one row to the next. `None`, the default, leaves `glReadPixels`
    /// writing nothing.
    fn pixels(&mut self) -> Option<(&'static [u32], usize)> {
        None
    }
    /// Points the scanout at the buffer whose first row is `row`, and
    /// shows the scanout.
    fn show(&mut self, row: u32);
    /// Waits for the vertical blanking to start, when the scanout takes
    /// the base it was given.
    fn wait_blanking(&mut self);
}
