// SPDX-License-Identifier: Apache-2.0
//! What an EGL swap needs of the machine it runs on (issue 996), alone,
//! so that a machine can be written without linking EGL's entry points:
//! a program that defines `#[no_mangle]` functions keeps every one, and
//! a program in the boot memory has no room for them.
#![no_std]

use razboj_tile::WORDS;

/// What a swap needs of the machine.
pub trait Machine {
    /// Razboj's display list, where GL writes a frame for it to read.
    fn list(&mut self) -> &'static mut [[u32; WORDS]];
    /// Has Razboj draw the first `entries` of the list, and waits until
    /// every pixel of them is written.
    fn draw(&mut self, entries: usize);
    /// Points the scanout at the buffer whose first row is `row`, and
    /// shows the scanout.
    fn show(&mut self, row: u32);
    /// Waits for the vertical blanking to start, when the scanout takes
    /// the base it was given.
    fn wait_blanking(&mut self);
}
