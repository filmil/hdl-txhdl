// SPDX-License-Identifier: Apache-2.0
//! The display list: what the rasteriser is told to draw.
//!
//! One entry is a box to walk and, when the entry is a triangle,
//! three vertices to test against. Everything a host can work out
//! once per primitive is worked out by the host: the box is already
//! clipped to the screen, and the vertices are already wound so that
//! the inside of the triangle is where all three edge functions are
//! non-negative. What is left for the hardware is the part that is
//! per pixel, which is the part worth building.
use txhdl::types::U;
use txhdl::{Transaction as TransactionDerive, Value as ValueDerive};

// begin{op}
/// One entry of a display list.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct Op {
    /// Fill every pixel of the box when zero, and test the three
    /// edges when one.
    pub tri: U<1>,
    /// The colour written, as `0xRRGGBB`.
    pub colour: U<24>,
    /// The box to walk, both ends included, clipped to the screen.
    pub x0: U<10>,
    pub y0: U<10>,
    pub x1: U<10>,
    pub y1: U<10>,
    /// The triangle's vertices, wound so that its inside is where
    /// every edge function is non-negative. Two's complement, so a
    /// vertex may lie off the screen on any side; see [`VMIN`].
    pub ax: U<12>,
    pub ay: U<12>,
    pub bx: U<12>,
    pub by: U<12>,
    pub cx: U<12>,
    pub cy: U<12>,
}
// end{op}

/// The range a vertex may take. A vertex is twelve bits of two's
/// complement, and the rasteriser's edge arithmetic is thirty-two
/// bits, which is wide enough for every product of two differences
/// of vertices in this range.
pub const VMIN: i32 = -2048;
pub const VMAX: i32 = 2047;

/// A vertex as it is stored: twelve bits of two's complement.
pub fn vertex(v: i32) -> U<12> {
    U::from((v & 0xfff) as u32)
}

/// A stored vertex read back as a number.
pub fn signed(v: U<12>) -> i32 {
    let r = v.raw() as i32;
    if r >= 2048 {
        r - 4096
    } else {
        r
    }
}

/// Twice the signed area of the triangle `a`, `b`, `c`: positive when
/// the three are wound the way the rasteriser wants.
fn area2(a: (i32, i32), b: (i32, i32), c: (i32, i32)) -> i32 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

impl Op {
    /// Fill the whole screen. A clear is a box with no triangle in it.
    pub fn clear(w: usize, h: usize, colour: u32) -> Self {
        Op::rect(0, 0, w as i32, h as i32, w, h, colour)
    }

    /// Fill a rectangle `w` by `h` at `x`, `y`, clipped to a screen of
    /// `sw` by `sh`. An empty rectangle yields `None`.
    #[allow(clippy::too_many_arguments)]
    pub fn rect_checked(
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        sw: usize,
        sh: usize,
        colour: u32,
    ) -> Option<Self> {
        let (x0, y0, x1, y1) = clip(x, y, x + w - 1, y + h - 1, sw, sh)?;
        Some(Op {
            tri: U::from(0u8),
            colour: U::from(colour),
            x0: U::from(x0),
            y0: U::from(y0),
            x1: U::from(x1),
            y1: U::from(y1),
            ..Op::default()
        })
    }

    /// The same, for a rectangle the caller knows is on the screen.
    #[allow(clippy::too_many_arguments)]
    pub fn rect(
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        sw: usize,
        sh: usize,
        colour: u32,
    ) -> Self {
        Op::rect_checked(x, y, w, h, sw, sh, colour)
            .expect("an empty rectangle")
    }

    /// A flat-shaded triangle, clipped to the screen and wound for the
    /// rasteriser. A degenerate or wholly off-screen triangle yields
    /// `None`.
    #[allow(clippy::too_many_arguments)]
    pub fn tri_checked(
        a: (i32, i32),
        b: (i32, i32),
        c: (i32, i32),
        sw: usize,
        sh: usize,
        colour: u32,
    ) -> Option<Self> {
        // The winding the rasteriser wants: swap two vertices when the
        // signed area says the other way.
        let (b, c) = if area2(a, b, c) < 0 { (c, b) } else { (b, c) };
        if area2(a, b, c) == 0 {
            return None;
        }
        // Every vertex must be in range, since the edge arithmetic is
        // sized for that range and for nothing wider.
        let ok = |p: (i32, i32)| {
            (VMIN..=VMAX).contains(&p.0) && (VMIN..=VMAX).contains(&p.1)
        };
        if !ok(a) || !ok(b) || !ok(c) {
            return None;
        }
        let lo = |f: fn((i32, i32)) -> i32| f(a).min(f(b)).min(f(c));
        let hi = |f: fn((i32, i32)) -> i32| f(a).max(f(b)).max(f(c));
        let (x0, y0, x1, y1) =
            clip(lo(|p| p.0), lo(|p| p.1), hi(|p| p.0), hi(|p| p.1), sw, sh)?;
        Some(Op {
            tri: U::from(1u8),
            colour: U::from(colour),
            x0: U::from(x0),
            y0: U::from(y0),
            x1: U::from(x1),
            y1: U::from(y1),
            ax: vertex(a.0),
            ay: vertex(a.1),
            bx: vertex(b.0),
            by: vertex(b.1),
            cx: vertex(c.0),
            cy: vertex(c.1),
        })
    }

    /// The same, for a triangle the caller knows is on the screen.
    pub fn tri(
        a: (i32, i32),
        b: (i32, i32),
        c: (i32, i32),
        sw: usize,
        sh: usize,
        colour: u32,
    ) -> Self {
        Op::tri_checked(a, b, c, sw, sh, colour).expect("a degenerate triangle")
    }
}

/// A box clipped to the screen, both ends included, or `None` when
/// nothing of it is on the screen.
fn clip(
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    sw: usize,
    sh: usize,
) -> Option<(u32, u32, u32, u32)> {
    let (x0, y0) = (x0.max(0), y0.max(0));
    let (x1, y1) = (x1.min(sw as i32 - 1), y1.min(sh as i32 - 1));
    if x1 < x0 || y1 < y0 {
        return None;
    }
    Some((x0 as u32, y0 as u32, x1 as u32, y1 as u32))
}
