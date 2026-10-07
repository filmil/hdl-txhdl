// SPDX-License-Identifier: Apache-2.0
//! Textures (issue 997): what a program and Razboj agree on, with no
//! standard library, so that the GL library on Vreteno lays textures out
//! with the same code the tests check.
//!
//! A texture is its levels in DDR3, each RGBA in 32 bits a texel,
//! `0xAARRGGBB` as a pixel is, and stored in blocks of 4 by 4 texels, 64
//! bytes, so that the texels a cache line holds are one burst. A
//! texture's descriptor, [`Desc`], is sixteen words the program keeps, on
//! a boundary of 64 bytes, and an entry names its texture by its
//! descriptor's byte address, so that the list says all Razboj reads.
//!
//! A textured entry carries three planes for the walk to step, as a
//! colour channel's: `u q` and `v q`, the texel coordinates of the base
//! level times `q`, with 32 bits of fraction, and `q`, the reciprocal of
//! the clip w scaled so that its largest at a vertex is one, with 48. All
//! three are affine in the window, so a pixel's `u` is `(u q) / q` there,
//! which is perspective-correct. The planes are 64 bits, so they take two
//! slots of their own after the entry's second slot:
//!
//! ```text
//!   slot A  words 0-5   u q: value, step right, step down, low word first
//!           words 6-11  v q: the same
//!           word 12     k, the shift of the numerators below
//!           word 13     the descriptor's byte address
//!           word 14     [2:0] the environment
//!           word 15     the environment's colour
//!   slot B  words 0-5   q: value, step right, step down, low word first
//!           words 6-13  the numerators of ∂u/∂x, ∂v/∂x, ∂u/∂y and ∂v/∂y,
//!                       each a value and a step, down for the first two
//!                       and right for the others, shifted right by k
//! ```
//!
//! The level of detail is GL's `ρ = max(|∂u/∂x|, |∂u/∂y|, |∂v/∂x|,
//! |∂v/∂y|)`, one of the scale factors GL ES allows. With `U = u q`,
//! `∂u/∂x = (U_x q - U q_x) / q²`, whose numerator varies only down the
//! box, and `∂u/∂y`'s only across it, so each is two words.

/// Words of a descriptor.
pub const DESC_WORDS: usize = 16;

/// The most levels a texture has: 1024 texels down to one.
pub const MAX_LEVELS: usize = 11;

/// The bit of an entry's word 15 that says it is textured, which gives it
/// its second slot and the two texture slots after that.
pub const TEXTURED: u32 = 1 << 14;

/// How the texels' channels are read, from the internal format GL was
/// given: the environments treat each differently.
pub const RGBA: u32 = 0;
pub const RGB: u32 = 1;
pub const ALPHA: u32 = 2;
pub const LUMINANCE: u32 = 3;
pub const LUMINANCE_ALPHA: u32 = 4;

/// The filters, as GL orders them: the magnification's two, and the
/// minification's six.
pub const NEAREST: u32 = 0;
pub const LINEAR: u32 = 1;
pub const NEAREST_MIPMAP_NEAREST: u32 = 2;
pub const LINEAR_MIPMAP_NEAREST: u32 = 3;
pub const NEAREST_MIPMAP_LINEAR: u32 = 4;
pub const LINEAR_MIPMAP_LINEAR: u32 = 5;

/// The environments, GL ES 1.1's without `COMBINE`.
pub const REPLACE: u32 = 0;
pub const MODULATE: u32 = 1;
pub const DECAL: u32 = 2;
pub const BLEND: u32 = 3;
pub const ADD: u32 = 4;

/// A texture's descriptor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Desc {
    /// The byte address of the base level.
    pub base: u32,
    /// The base level's width and height, as their log2.
    pub log_w: u32,
    pub log_h: u32,
    /// How many levels there are, from one.
    pub levels: u32,
    /// Whether each coordinate is clamped to the edge rather than
    /// repeated.
    pub clamp_s: bool,
    pub clamp_t: bool,
    /// The minification's filter and the magnification's.
    pub min: u32,
    pub mag: u32,
    /// How the texels are read, [`RGBA`] to [`LUMINANCE_ALPHA`].
    pub class: u32,
}

/// A level's side, from the base's log2 side: half the next level up, and
/// never below one.
pub fn side(log: u32, level: u32) -> u32 {
    1 << log.saturating_sub(level)
}

/// How many blocks of 4 a level's side takes: a quarter, and at least one.
pub fn blocks(log: u32, level: u32) -> u32 {
    1 << log.saturating_sub(level).saturating_sub(2)
}

/// A level's bytes, its blocks of 64.
pub fn level_bytes(d: &Desc, level: u32) -> u32 {
    blocks(d.log_w, level) * blocks(d.log_h, level) * 64
}

/// The byte address of a level, the levels one after another from the
/// base.
pub fn level_base(d: &Desc, level: u32) -> u32 {
    d.base + (0..level).map(|l| level_bytes(d, l)).sum::<u32>()
}

/// The byte offset of the texel `(i, j)` in a level whose side is
/// `log_w`'s at that level, in its blocks of 4 by 4.
pub fn texel_offset(d: &Desc, level: u32, i: u32, j: u32) -> u32 {
    let across = blocks(d.log_w, level);
    ((j >> 2) * across + (i >> 2)) * 64 + ((j & 3) * 4 + (i & 3)) * 4
}

/// A descriptor as its sixteen words: word 0 its sizes and modes, and
/// words 1 to 11 each level's byte address.
///
/// ```text
///   word 0   [3:0] log2 w   [7:4] log2 h   [11:8] levels
///            [12] clamp s   [13] clamp t   [18:16] min   [19] mag
///            [22:20] class
///   word 1+l the byte address of level l
/// ```
pub fn encode(d: &Desc) -> [u32; DESC_WORDS] {
    let mut w = [0u32; DESC_WORDS];
    w[0] = (d.log_w & 0xf)
        | (d.log_h & 0xf) << 4
        | (d.levels & 0xf) << 8
        | (d.clamp_s as u32) << 12
        | (d.clamp_t as u32) << 13
        | (d.min & 7) << 16
        | (d.mag & 1) << 19
        | (d.class & 7) << 20;
    for l in 0..MAX_LEVELS as u32 {
        w[1 + l as usize] = level_base(d, l.min(d.levels.max(1) - 1));
    }
    w
}

/// A descriptor read back from its words.
pub fn decode(w: &[u32; DESC_WORDS]) -> Desc {
    Desc {
        base: w[1],
        log_w: w[0] & 0xf,
        log_h: (w[0] >> 4) & 0xf,
        levels: (w[0] >> 8) & 0xf,
        clamp_s: (w[0] >> 12) & 1 == 1,
        clamp_t: (w[0] >> 13) & 1 == 1,
        min: (w[0] >> 16) & 7,
        mag: (w[0] >> 19) & 1,
        class: (w[0] >> 20) & 7,
    }
}

/// A 64-bit plane in a slot, low word first: value, step right, step down.
pub fn plane_words(p: [u64; 3]) -> [u32; 6] {
    let mut w = [0u32; 6];
    for (k, v) in p.iter().enumerate() {
        w[2 * k] = *v as u32;
        w[2 * k + 1] = (*v >> 32) as u32;
    }
    w
}

/// A 64-bit plane read back from six words.
pub fn plane_of(w: &[u32]) -> [u64; 3] {
    core::array::from_fn(|k| w[2 * k] as u64 | (w[2 * k + 1] as u64) << 32)
}

/// A 64-bit plane stepped `i` pixels right and `j` rows down, wrapping as
/// the rasteriser's adders do.
pub fn step_plane(p: [u64; 3], i: u32, j: u32) -> [u64; 3] {
    let v = p[0]
        .wrapping_add(p[1].wrapping_mul(i as u64))
        .wrapping_add(p[2].wrapping_mul(j as u64));
    [v, p[1], p[2]]
}

/// A textured entry's two slots with their planes stepped `i` pixels
/// right and `j` rows down, for an entry clipped to a tile.
pub fn step_slots(
    a: &[u32; 16],
    b: &[u32; 16],
    i: u32,
    j: u32,
) -> [[u32; 16]; 2] {
    let (mut a2, mut b2) = (*a, *b);
    let put = |w: &mut [u32; 16], at: usize, p: [u64; 3]| {
        w[at..at + 6].copy_from_slice(&plane_words(step_plane(p, i, j)));
    };
    put(&mut a2, 0, plane_of(&a[0..6]));
    put(&mut a2, 6, plane_of(&a[6..12]));
    put(&mut b2, 0, plane_of(&b[0..6]));
    // The numerators: the first two step down, the others right.
    for (at, by) in [(6, j), (8, j), (10, i), (12, i)] {
        b2[at] = b[at].wrapping_add(b[at + 1].wrapping_mul(by));
    }
    [a2, b2]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout: a level's blocks of 4 by 4, row after row of blocks,
    /// and the levels one after another; a level smaller than a block
    /// takes one.
    #[test]
    fn the_levels_and_their_blocks() {
        let d = Desc {
            base: 0x1000,
            log_w: 3,
            log_h: 2,
            levels: 4,
            ..Desc::default()
        };
        // 8 by 4: two blocks across, one down.
        assert_eq!(level_bytes(&d, 0), 128);
        assert_eq!(texel_offset(&d, 0, 5, 2), 64 + (2 * 4 + 1) * 4);
        // 4 by 2, 2 by 1 and 1 by 1: one block each.
        assert_eq!(level_bytes(&d, 1), 64);
        assert_eq!(level_bytes(&d, 3), 64);
        assert_eq!(level_base(&d, 2), 0x1000 + 128 + 64);
        assert_eq!(side(3, 4), 1);
    }

    /// A descriptor survives its words.
    #[test]
    fn a_descriptor_survives_its_words() {
        let d = Desc {
            base: 0x4000,
            log_w: 6,
            log_h: 5,
            levels: 7,
            clamp_s: true,
            clamp_t: false,
            min: LINEAR_MIPMAP_LINEAR,
            mag: LINEAR,
            class: LUMINANCE_ALPHA,
        };
        let w = encode(&d);
        assert_eq!(decode(&w), d);
        assert_eq!(w[1 + 6], level_base(&d, 6));
    }

    /// A plane stepped, wrapping, and its words.
    #[test]
    fn a_plane_steps_and_wraps() {
        let p = [u64::MAX - 1, 3, 1 << 40];
        assert_eq!(step_plane(p, 1, 2)[0], 1 + (1u64 << 41));
        assert_eq!(plane_of(&plane_words(p)), p);
    }
}
