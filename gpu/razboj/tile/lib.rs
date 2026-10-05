// SPDX-License-Identifier: Apache-2.0
//! Razboj's display list, for the CPU, without the standard library: the
//! clip of a box to the screen or to a scissor box, which the assembler
//! uses (issue 990), in a crate a Vreteno program can use too, so that
//! the binning into tiles shares it (issue 1157).
#![cfg_attr(not(test), no_std)]

// begin{clip}
/// A box of pixels, both ends included: the first column and row, then
/// the last.
pub type Bounds = (u32, u32, u32, u32);

/// A box clipped to `within`, the screen, a scissor box on it or a
/// tile, both ends included, or `None` when nothing of it is inside.
pub fn clip(
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    within: Bounds,
) -> Option<Bounds> {
    let (wx0, wy0, wx1, wy1) = within;
    let (x0, y0) = (x0.max(wx0 as i32), y0.max(wy0 as i32));
    let (x1, y1) = (x1.min(wx1 as i32), y1.min(wy1 as i32));
    if x1 < x0 || y1 < y0 {
        return None;
    }
    Some((x0 as u32, y0 as u32, x1 as u32, y1 as u32))
}
// end{clip}
