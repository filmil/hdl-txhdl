// SPDX-License-Identifier: Apache-2.0
//! `glGetIntegerv`, `glGetFixedv` and `glGetBooleanv` (#1484): the state
//! a context keeps, and the limits of this implementation, by GL's names
//! for them.
//!
//! A query finds its values once, in the kind GL keeps them in, and each
//! of the three calls converts them as GL ES 1.1's section 6.1.2 says:
//! a boolean is one or nought; a fixed value asked for as an integer is
//! rounded to the nearest, except a colour, a normal, the depth range,
//! the depth's clear value and the alpha test's reference, which map
//! from minus one to one onto the integers' whole range; an integer asked
//! for as fixed is that many ones; and anything not nought is true.

use crate::fixed::{Fx, ONE};
use crate::{gl, Gl};

/// The names this answers, GL ES 1.1's.
pub mod name {
    pub const CURRENT_COLOR: u32 = 0x0B00;
    pub const CURRENT_NORMAL: u32 = 0x0B02;
    pub const CURRENT_TEXTURE_COORDS: u32 = 0x0B03;
    pub const POINT_SIZE: u32 = 0x0B11;
    pub const SMOOTH_POINT_SIZE_RANGE: u32 = 0x0B12;
    pub const LINE_WIDTH: u32 = 0x0B21;
    pub const SMOOTH_LINE_WIDTH_RANGE: u32 = 0x0B22;
    pub const CULL_FACE_MODE: u32 = 0x0B45;
    pub const FRONT_FACE: u32 = 0x0B46;
    pub const SHADE_MODEL: u32 = 0x0B54;
    pub const DEPTH_RANGE: u32 = 0x0B70;
    pub const DEPTH_WRITEMASK: u32 = 0x0B72;
    pub const DEPTH_CLEAR_VALUE: u32 = 0x0B73;
    pub const DEPTH_FUNC: u32 = 0x0B74;
    pub const MATRIX_MODE: u32 = 0x0BA0;
    pub const SCISSOR_BOX: u32 = 0x0C10;
    pub const VIEWPORT: u32 = 0x0BA2;
    pub const MODELVIEW_STACK_DEPTH: u32 = 0x0BA3;
    pub const PROJECTION_STACK_DEPTH: u32 = 0x0BA4;
    pub const TEXTURE_STACK_DEPTH: u32 = 0x0BA5;
    pub const MODELVIEW_MATRIX: u32 = 0x0BA6;
    pub const PROJECTION_MATRIX: u32 = 0x0BA7;
    pub const TEXTURE_MATRIX: u32 = 0x0BA8;
    pub const ALPHA_TEST_FUNC: u32 = 0x0BC1;
    pub const ALPHA_TEST_REF: u32 = 0x0BC2;
    pub const BLEND_DST: u32 = 0x0BE0;
    pub const BLEND_SRC: u32 = 0x0BE1;
    pub const COLOR_CLEAR_VALUE: u32 = 0x0C22;
    pub const COLOR_WRITEMASK: u32 = 0x0C23;
    pub const UNPACK_ALIGNMENT: u32 = 0x0CF5;
    pub const MAX_LIGHTS: u32 = 0x0D31;
    pub const MAX_CLIP_PLANES: u32 = 0x0D32;
    pub const MAX_TEXTURE_SIZE: u32 = 0x0D33;
    pub const MAX_MODELVIEW_STACK_DEPTH: u32 = 0x0D36;
    pub const MAX_PROJECTION_STACK_DEPTH: u32 = 0x0D38;
    pub const MAX_TEXTURE_STACK_DEPTH: u32 = 0x0D39;
    pub const MAX_VIEWPORT_DIMS: u32 = 0x0D3A;
    pub const SUBPIXEL_BITS: u32 = 0x0D50;
    pub const RED_BITS: u32 = 0x0D52;
    pub const GREEN_BITS: u32 = 0x0D53;
    pub const BLUE_BITS: u32 = 0x0D54;
    pub const ALPHA_BITS: u32 = 0x0D55;
    pub const DEPTH_BITS: u32 = 0x0D56;
    pub const STENCIL_BITS: u32 = 0x0D57;
    pub const TEXTURE_BINDING_2D: u32 = 0x8069;
    pub const ACTIVE_TEXTURE: u32 = 0x84E0;
    pub const MAX_TEXTURE_UNITS: u32 = 0x84E2;
    pub const ALIASED_POINT_SIZE_RANGE: u32 = 0x846D;
    pub const ALIASED_LINE_WIDTH_RANGE: u32 = 0x846E;
    pub const NUM_COMPRESSED_TEXTURE_FORMATS: u32 = 0x86A2;
    pub const COMPRESSED_TEXTURE_FORMATS: u32 = 0x86A3;
    pub const TEXTURE0: u32 = 0x84C0;
}

/// How GL keeps a query's values, which sets how each call converts
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Integer,
    Fixed,
    /// Fixed, and mapped onto the integers' whole range when asked for
    /// as integers: colours, normals, depths and the alpha reference.
    Unit,
    Boolean,
}

/// A query's values: up to sixteen, a matrix's.
#[derive(Clone, Copy, Debug)]
pub struct Got {
    pub kind: Kind,
    pub values: [i64; 16],
    pub len: usize,
}

impl Got {
    fn of(kind: Kind, v: &[i64]) -> Got {
        let mut values = [0i64; 16];
        values[..v.len()].copy_from_slice(v);
        Got {
            kind,
            values,
            len: v.len(),
        }
    }

    /// As `glGetIntegerv` gives them.
    pub fn integers(&self, out: &mut [i32]) {
        for (o, &v) in out.iter_mut().zip(&self.values[..self.len]) {
            *o = match self.kind {
                Kind::Integer | Kind::Boolean => v as i32,
                Kind::Fixed => ((v + (ONE as i64 / 2)) >> 16) as i32,
                // [-1, 1] onto [-(2^31 - 1), 2^31 - 1].
                Kind::Unit => {
                    let m = i32::MAX as i64;
                    let x = (v * (2 * m + 1) - ONE as i64) / (2 * ONE as i64);
                    x.clamp(-m, m) as i32
                }
            };
        }
    }

    /// As `glGetFixedv` gives them.
    pub fn fixed(&self, out: &mut [Fx]) {
        for (o, &v) in out.iter_mut().zip(&self.values[..self.len]) {
            *o = match self.kind {
                Kind::Integer | Kind::Boolean => {
                    (v << 16).clamp(i32::MIN as i64, i32::MAX as i64) as Fx
                }
                Kind::Fixed | Kind::Unit => v as Fx,
            };
        }
    }

    /// As `glGetBooleanv` gives them.
    pub fn booleans(&self, out: &mut [bool]) {
        for (o, &v) in out.iter_mut().zip(&self.values[..self.len]) {
            *o = v != 0;
        }
    }
}

impl Gl<'_> {
    /// The values GL keeps for `pname`, or `None` for a name GL ES 1.1
    /// does not have or this library does not keep.
    pub fn get(&self, pname: u32) -> Option<Got> {
        use name::*;
        use Kind::*;
        let i = |v: &[i64]| Some(Got::of(Integer, v));
        let f = |v: &[Fx]| {
            let w: [i64; 16] =
                core::array::from_fn(|k| *v.get(k).unwrap_or(&0) as i64);
            Some(Got::of(Fixed, &w[..v.len()]))
        };
        let u = |v: &[Fx]| {
            let w: [i64; 16] =
                core::array::from_fn(|k| *v.get(k).unwrap_or(&0) as i64);
            Some(Got::of(Unit, &w[..v.len()]))
        };
        let b = |v: &[bool]| {
            let w: [i64; 16] =
                core::array::from_fn(|k| *v.get(k).unwrap_or(&false) as i64);
            Some(Got::of(Boolean, &w[..v.len()]))
        };
        let factor = |code: u32| match code {
            0 => gl::ZERO,
            1 => gl::ONE,
            c => gl::SRC_COLOR + c - 2,
        };
        let size = gl::MAX_SIZE as i64;
        let mask = self.colour_mask;
        match pname {
            CURRENT_COLOR => u(&self.colour),
            CURRENT_NORMAL => u(&self.normal),
            CURRENT_TEXTURE_COORDS => f(&self.tex_coords),
            POINT_SIZE => f(&[self.point_size]),
            LINE_WIDTH => f(&[self.line_width]),
            ALIASED_POINT_SIZE_RANGE | ALIASED_LINE_WIDTH_RANGE => {
                i(&[1, size])
            }
            SMOOTH_POINT_SIZE_RANGE | SMOOTH_LINE_WIDTH_RANGE => i(&[1, 1]),
            CULL_FACE_MODE => i(&[self.cull as i64]),
            FRONT_FACE => i(&[self.front as i64]),
            SHADE_MODEL => {
                i(&[if self.smooth { gl::SMOOTH } else { gl::FLAT } as i64])
            }
            DEPTH_RANGE => u(&[self.depth_range.0, self.depth_range.1]),
            DEPTH_WRITEMASK => b(&[self.depth_mask]),
            DEPTH_CLEAR_VALUE => u(&[self.clear_depth]),
            DEPTH_FUNC => i(&[(gl::NEVER + self.depth_func) as i64]),
            MATRIX_MODE => i(&[self.mode as i64]),
            VIEWPORT => {
                let (x, y, w, h) = self.viewport;
                i(&[x as i64, y as i64, w as i64, h as i64])
            }
            MODELVIEW_STACK_DEPTH => i(&[self.mv_top as i64 + 1]),
            PROJECTION_STACK_DEPTH => i(&[self.pj_top as i64 + 1]),
            TEXTURE_STACK_DEPTH => i(&[self.tx_top as i64 + 1]),
            MODELVIEW_MATRIX => f(&self.mv[self.mv_top]),
            PROJECTION_MATRIX => f(&self.pj[self.pj_top]),
            TEXTURE_MATRIX => f(&self.tx[self.tx_top]),
            ALPHA_TEST_FUNC => i(&[(gl::NEVER + self.alpha.0) as i64]),
            ALPHA_TEST_REF => {
                let r = self.alpha.1 as i64;
                u(&[((r * ONE as i64 + 127) / 255) as Fx])
            }
            BLEND_SRC => i(&[factor(self.blend.0) as i64]),
            BLEND_DST => i(&[factor(self.blend.1) as i64]),
            COLOR_CLEAR_VALUE => u(&self.clear_colour),
            // Red, green, blue and alpha; the mask keeps blue in bit 0.
            COLOR_WRITEMASK => {
                b(
                    &[
                        mask & 4 != 0,
                        mask & 2 != 0,
                        mask & 1 != 0,
                        mask & 8 != 0,
                    ],
                )
            }
            UNPACK_ALIGNMENT => i(&[self.unpack as i64]),
            MAX_LIGHTS => i(&[gl::MAX_LIGHTS as i64]),
            MAX_CLIP_PLANES => i(&[1]),
            MAX_TEXTURE_SIZE => i(&[gl::MAX_TEXTURE_SIZE as i64]),
            MAX_MODELVIEW_STACK_DEPTH => {
                i(&[gl::MAX_MODELVIEW_STACK_DEPTH as i64])
            }
            MAX_PROJECTION_STACK_DEPTH => {
                i(&[gl::MAX_PROJECTION_STACK_DEPTH as i64])
            }
            MAX_TEXTURE_STACK_DEPTH => i(&[gl::MAX_TEXTURE_STACK_DEPTH as i64]),
            // A vertex may lie 1024 pixels either side of the origin.
            MAX_VIEWPORT_DIMS => i(&[1024, 1024]),
            SUBPIXEL_BITS => i(&[4]),
            RED_BITS | GREEN_BITS | BLUE_BITS | ALPHA_BITS => i(&[8]),
            DEPTH_BITS => i(&[16]),
            STENCIL_BITS => i(&[0]),
            TEXTURE_BINDING_2D => i(&[self.bound as i64]),
            ACTIVE_TEXTURE => i(&[TEXTURE0 as i64]),
            MAX_TEXTURE_UNITS => i(&[1]),
            // The ten paletted formats (#998), GL's values in GL's order.
            NUM_COMPRESSED_TEXTURE_FORMATS => i(&[10]),
            COMPRESSED_TEXTURE_FORMATS => {
                let first = gl::PALETTE4_RGB8_OES as i64;
                let v: [i64; 10] = core::array::from_fn(|k| first + k as i64);
                i(&v)
            }
            gl::LIGHT_MODEL_AMBIENT => u(&self.scene_ambient),
            gl::LIGHT_MODEL_TWO_SIDE => b(&[self.two_side]),
            // Polygon offset (#998).
            // The scissor and the hints (#1490).
            SCISSOR_BOX => {
                let (x, y, w, h) = self.scissor;
                i(&[x as i64, y as i64, w as i64, h as i64])
            }
            h if crate::HINTS.contains(&h) => {
                let k = crate::HINTS.iter().position(|&t| t == h).unwrap_or(0);
                i(&[self.hints[k] as i64])
            }
            gl::POLYGON_OFFSET_FACTOR => f(&[self.offset.0]),
            gl::POLYGON_OFFSET_UNITS => f(&[self.offset.1]),
            cap => self.enabled(cap).and_then(|on| b(&[on])),
        }
    }
}
