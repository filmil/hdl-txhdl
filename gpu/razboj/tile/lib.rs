// SPDX-License-Identifier: Apache-2.0
//! Razboj's display list in tiles, for the CPU: the tile table's format,
//! and the binning that sorts a list's entries into the tiles they
//! touch (`docs/razboj-tiles.md`, issue 1157).
//!
//! This crate has no standard library and allocates nothing, so a
//! Vreteno program can bin its own lists with the same code the tests
//! check against the model. It works on instructions as the words of
//! `razboj::dl`'s format, after the assembler has clipped, wound and
//! shaded them, and it writes words in the same format back.
//!
//! A tile is 64 by 64 pixels, its origin at multiples of 64. A tile is
//! a scissor box, so binning an entry is clipping its box to each tile
//! it touches, with [`clip`], the clip the assembler's scissor box
//! uses (issue 990), and writing one entry per tile. The edges need
//! nothing more, since the rasteriser tests them against the same
//! vertices whatever the box. A shaded triangle's planes start at its
//! box's first pixel, so a clipped entry's start is the whole entry's
//! stepped to the clipped box's first pixel, in the wrapping
//! thirty-two bit arithmetic of the rasteriser's own adders: then each
//! pixel's colour is the one the untiled entry gives it, to the bit.
//! Working each tile's planes out afresh would round at a different
//! place and could differ in the last bit.
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

// begin{format}
/// Words an instruction takes: `razboj::dl::WORDS`, which that crate
/// checks against this one.
pub const WORDS: usize = 16;

/// A tile's side, a power of two: 64 pixels, so that a pixel's place in
/// its tile is the low six bits of each of its coordinates.
pub const TILE_SHIFT: u32 = 6;
pub const TILE: u32 = 1 << TILE_SHIFT;

/// The most tiles a row and a column hold: a screen of 1024 by 1024,
/// the most an instruction's ten bits of coordinate reach.
pub const MAX_COLS: usize = 1024 >> TILE_SHIFT;
pub const MAX_TILES: usize = MAX_COLS * MAX_COLS;

/// Words a tile's record takes in the tile table, a power of two, so
/// that the `t`th record is at `base + (t << TILE_BYTE_SHIFT)`.
pub const TILE_WORDS: usize = 2;
pub const TILE_BYTE_SHIFT: usize = 3;

/// Where a tiled list's entries start, in bytes past its tile table:
/// room for every tile's record, so a program lays out the table and
/// the entries the same way whatever the screen (issue 1255).
pub const ENTRIES_AT: usize = MAX_TILES << TILE_BYTE_SHIFT;

/// The count word's bit that says the list is a tile table, its count
/// then the number of tiles, rather than a flat list of entries.
pub const TILED: u32 = 1 << 31;

/// A tile's record in the tile table:
///
/// ```text
///   word 0  [15:0] first     [31:16] count
///   word 1   [9:0] x origin  [25:16] y origin
/// ```
///
/// `first` is the index of the tile's first entry in the binned list,
/// where its entries are `count` instructions one after another, and
/// the origin is the tile's top left pixel, multiples of 64. The table
/// holds only the tiles some entry touches, left to right and top to
/// bottom, so a pixel that no entry covers is not written, as it is not
/// in the untiled list either; its length takes the place of the count
/// the rasteriser polls for.
pub fn record(first: u32, count: u32, x: u32, y: u32) -> [u32; TILE_WORDS] {
    [
        (first & 0xffff) | (count << 16),
        (x & 0x3ff) | ((y & 0x3ff) << 16),
    ]
}
// end{format}

/// The instruction kinds, as word 0's low two bits say them.
const CLEAR: u32 = 0;
const RECT: u32 = 1;
const SHADED: u32 = 3;

/// The box an instruction walks: its own, or the screen's for a clear,
/// which the rasteriser supplies.
fn walked(e: &[u32; WORDS], sw: u32, sh: u32) -> Bounds {
    if e[0] & 3 == CLEAR {
        return (0, 0, sw - 1, sh - 1);
    }
    let lo = |w: u32| w & 0x3ff;
    let hi = |w: u32| (w >> 16) & 0x3ff;
    (lo(e[1]), hi(e[1]), lo(e[2]), hi(e[2]))
}

/// The tile `(i, j)`, `i` across and `j` down, on a screen of `sw` by
/// `sh` pixels: its box, which the screen's edge cuts short in the last
/// row and column when the screen is not a whole number of tiles.
pub fn tile(i: u32, j: u32, sw: u32, sh: u32) -> Bounds {
    let (x, y) = (i << TILE_SHIFT, j << TILE_SHIFT);
    (x, y, (x + TILE - 1).min(sw - 1), (y + TILE - 1).min(sh - 1))
}

// begin{entry}
/// An entry of a screen `sw` by `sh` clipped to the box `within`, or
/// `None` when it has no pixel there. A clear becomes a rectangle of
/// the box, since a clear's box is always the whole screen. A shaded
/// triangle's planes are stepped from its box's first pixel to the
/// clipped box's, as the rasteriser's adders would have stepped them.
pub fn clip_entry(
    e: &[u32; WORDS],
    within: Bounds,
    sw: u32,
    sh: u32,
) -> Option<[u32; WORDS]> {
    let (x0, y0, x1, y1) = walked(e, sw, sh);
    let (cx0, cy0, cx1, cy1) =
        clip(x0 as i32, y0 as i32, x1 as i32, y1 as i32, within)?;
    let mut out = *e;
    if e[0] & 3 == CLEAR {
        out[0] = (e[0] & !3) | RECT;
    }
    out[1] = cx0 | (cy0 << 16);
    out[2] = cx1 | (cy1 << 16);
    if e[0] & 3 == SHADED {
        let (i, j) = (cx0 - x0, cy0 - y0);
        for ch in [6, 9, 12] {
            let (start, dx, dy) = (e[ch], e[ch + 1], e[ch + 2]);
            out[ch] = start
                .wrapping_add(dx.wrapping_mul(i))
                .wrapping_add(dy.wrapping_mul(j));
        }
    }
    Some(out)
}
// end{entry}

/// What a binning wrote: how many tile records, and how many entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binned {
    pub tiles: usize,
    pub entries: usize,
}

/// Why a binning wrote nothing: the room it was given was too small,
/// for the tile records or for the entries, or the screen is larger
/// than an instruction's coordinates reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    Tiles,
    Entries,
    Screen,
}

// begin{bin}
/// A display list of a screen `sw` by `sh` binned into tiles: each
/// tile's entries, in the list's order, one tile after another, into
/// `entries`, and the tiles' records into `tiles`, in the order of
/// [`record`]. Each entry goes into every tile its box touches, clipped
/// to the tile by [`clip_entry`].
///
/// Two passes over the list, and no allocation: the first counts each
/// tile's entries, so each tile's place is known, and the second writes
/// them there. Nothing is written when the room is too small.
pub fn bin(
    list: &[[u32; WORDS]],
    sw: u32,
    sh: u32,
    entries: &mut [[u32; WORDS]],
    tiles: &mut [[u32; TILE_WORDS]],
) -> Result<Binned, Refused> {
    if sw == 0 || sh == 0 || sw > 1024 || sh > 1024 {
        return Err(Refused::Screen);
    }
    let cols = ((sw + TILE - 1) >> TILE_SHIFT) as usize;
    let span = |e: &[u32; WORDS]| {
        let (x0, y0, x1, y1) = walked(e, sw, sh);
        let (x1, y1) = (x1.min(sw - 1), y1.min(sh - 1));
        (
            x0 >> TILE_SHIFT,
            y0 >> TILE_SHIFT,
            x1 >> TILE_SHIFT,
            y1 >> TILE_SHIFT,
        )
    };
    // How many entries each tile takes.
    let mut count = [0u32; MAX_TILES];
    for e in list {
        let (i0, j0, i1, j1) = span(e);
        for j in j0..=j1 {
            for i in i0..=i1 {
                count[j as usize * cols + i as usize] += 1;
            }
        }
    }
    // Whether it all fits: the room given, and the sixteen bits of a
    // record's first entry and count.
    let used = count.iter().filter(|&&c| c > 0).count();
    let total: u32 = count.iter().sum();
    if used > tiles.len() {
        return Err(Refused::Tiles);
    }
    if total as usize > entries.len() || total > 0xffff {
        return Err(Refused::Entries);
    }
    // Where each tile's entries start, and the records.
    let mut at = [0u32; MAX_TILES];
    let (mut n, mut t) = (0u32, 0usize);
    for (k, &c) in count.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let (i, j) = ((k % cols) as u32, (k / cols) as u32);
        tiles[t] = record(n, c, i << TILE_SHIFT, j << TILE_SHIFT);
        at[k] = n;
        n += c;
        t += 1;
    }
    // The entries, each clipped into every tile it touches.
    for e in list {
        let (i0, j0, i1, j1) = span(e);
        for j in j0..=j1 {
            for i in i0..=i1 {
                let k = j as usize * cols + i as usize;
                let within = tile(i, j, sw, sh);
                let clipped = clip_entry(e, within, sw, sh).expect(
                    "a tile the entry's box reaches holds a pixel of it",
                );
                entries[at[k] as usize] = clipped;
                at[k] += 1;
            }
        }
    }
    Ok(Binned {
        tiles: t,
        entries: n as usize,
    })
}
// end{bin}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rectangle of `colour` over the box, as the format writes it.
    fn rect(colour: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> [u32; WORDS] {
        let mut w = [0u32; WORDS];
        w[0] = RECT | (colour << 2);
        w[1] = x0 | (y0 << 16);
        w[2] = x1 | (y1 << 16);
        w
    }

    #[test]
    fn a_record_is_two_words_of_fields() {
        assert_eq!(record(3, 5, 128, 64), [3 | (5 << 16), 128 | (64 << 16)]);
        assert_eq!(1 << TILE_BYTE_SHIFT, TILE_WORDS * 4);
    }

    /// The last row and column of tiles stop at the screen's edge.
    #[test]
    fn a_tile_stops_at_the_screens_edge() {
        assert_eq!(tile(0, 0, 640, 480), (0, 0, 63, 63));
        assert_eq!(tile(9, 7, 640, 480), (576, 448, 639, 479));
    }

    /// A clear becomes a rectangle of the tile, its colour kept.
    #[test]
    fn a_clear_becomes_a_rectangle_of_the_tile() {
        let mut clear = [0u32; WORDS];
        clear[0] = CLEAR | (0x12_3456 << 2);
        clear[15] = 0x80;
        let got = clip_entry(&clear, tile(2, 1, 640, 480), 640, 480).unwrap();
        let mut want = rect(0x12_3456, 128, 64, 191, 127);
        want[15] = 0x80;
        assert_eq!(got, want);
    }

    /// A box that misses the tile has nothing in it.
    #[test]
    fn a_box_outside_the_tile_is_nothing() {
        let r = rect(1, 0, 0, 10, 10);
        assert_eq!(clip_entry(&r, tile(1, 0, 640, 480), 640, 480), None);
    }

    /// A shaded triangle's start moves to the clipped box's first pixel
    /// by its steps, wrapping, and the steps stay.
    #[test]
    fn a_plane_is_stepped_to_the_tiles_first_pixel() {
        let mut e = rect(0, 50, 60, 100, 70);
        e[0] = SHADED;
        let (start, dx, dy) = (0x0010_0000u32, 0xffff_8000u32, 0x0002_0000u32);
        e[6] = start;
        e[7] = dx;
        e[8] = dy;
        let got = clip_entry(&e, tile(1, 1, 640, 480), 640, 480).unwrap();
        // The clipped box starts 14 right and 4 down of the whole one.
        assert_eq!((got[1], got[2]), (64 | (64 << 16), 100 | (70 << 16)));
        let want = start
            .wrapping_add(dx.wrapping_mul(14))
            .wrapping_add(dy.wrapping_mul(4));
        assert_eq!((got[6], got[7], got[8]), (want, dx, dy));
    }

    /// Each tile takes the entries that touch it, in the list's order,
    /// and the tiles come left to right and top to bottom; a tile no
    /// entry touches has no record.
    #[test]
    fn entries_go_to_the_tiles_they_touch_in_order() {
        let list = [rect(1, 60, 10, 70, 20), rect(2, 0, 0, 5, 5)];
        let mut entries = [[0u32; WORDS]; 8];
        let mut tiles = [[0u32; TILE_WORDS]; 8];
        let got = bin(&list, 640, 480, &mut entries, &mut tiles).unwrap();
        assert_eq!(
            got,
            Binned {
                tiles: 2,
                entries: 3
            }
        );
        assert_eq!(tiles[0], record(0, 2, 0, 0));
        assert_eq!(tiles[1], record(2, 1, 64, 0));
        assert_eq!(entries[0], rect(1, 60, 10, 63, 20));
        assert_eq!(entries[1], rect(2, 0, 0, 5, 5));
        assert_eq!(entries[2], rect(1, 64, 10, 70, 20));
    }

    /// Too little room, or too large a screen, and nothing is written.
    #[test]
    fn too_little_room_writes_nothing() {
        let list = [rect(1, 60, 10, 70, 20)];
        let mut entries = [[7u32; WORDS]; 1];
        let mut tiles = [[7u32; TILE_WORDS]; 4];
        let r = bin(&list, 640, 480, &mut entries, &mut tiles);
        assert_eq!(r, Err(Refused::Entries));
        let mut few = [[7u32; TILE_WORDS]; 1];
        let mut room = [[7u32; WORDS]; 4];
        assert_eq!(
            bin(&list, 640, 480, &mut room, &mut few),
            Err(Refused::Tiles)
        );
        assert_eq!(
            bin(&list, 2048, 480, &mut room, &mut tiles),
            Err(Refused::Screen)
        );
        assert!(entries.iter().chain(room.iter()).all(|e| e == &[7; WORDS]));
        assert!(tiles
            .iter()
            .chain(few.iter())
            .all(|t| t == &[7; TILE_WORDS]));
    }
}
