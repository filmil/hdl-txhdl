// SPDX-License-Identifier: Apache-2.0
//! Lighting: GL ES 1.1's sum at a vertex, in eye space and in 16.16
//! (`docs/gles.md`, section 5; issue 1164).
//!
//! A vertex's colour is the material's emission, plus the scene's
//! ambient times the material's, plus for each enabled light its
//! attenuation times its spot factor times the sum of three terms: the
//! light's ambient times the material's; the diffuse, the normal against
//! the direction to the light; and the specular, the normal against the
//! half vector raised to the shininess, which counts only where the
//! diffuse does. The sum is clamped to nought to one, and the alpha is
//! the material's diffuse alpha. The viewer is at infinity, as GL's
//! default has it, so the half vector is the direction to the light plus
//! (0, 0, 1).
//!
//! A directional light, `w` nought in its position, needs no distance;
//! a positional one needs a square root and a division a vertex for its
//! attenuation, which is one when its two higher terms are nought, as by
//! default.

use crate::fixed::{div, isqrt, mul, narrow, pow, sin_cos, Fx, ONE};

/// A light's state, its position and spot direction in eye space as the
/// modelview carried them when they were set.
#[derive(Clone, Copy, Debug)]
pub struct Light {
    pub ambient: [Fx; 4],
    pub diffuse: [Fx; 4],
    pub specular: [Fx; 4],
    pub position: [Fx; 4],
    pub spot_direction: [Fx; 3],
    pub spot_exponent: Fx,
    pub spot_cutoff: Fx,
    /// Constant, linear and quadratic attenuation.
    pub attenuation: [Fx; 3],
    pub on: bool,
}

impl Light {
    /// Light `i` as the specification starts it: light 0 white and the
    /// others black, each directional along the eye's z, off.
    pub fn new(i: usize) -> Self {
        let lit = if i == 0 { ONE } else { 0 };
        Light {
            ambient: [0, 0, 0, ONE],
            diffuse: [lit, lit, lit, ONE],
            specular: [lit, lit, lit, ONE],
            position: [0, 0, ONE, 0],
            spot_direction: [0, 0, -ONE],
            spot_exponent: 0,
            spot_cutoff: 180 * ONE,
            attenuation: [ONE, 0, 0],
            on: false,
        }
    }
}

/// A material, as the specification starts it.
#[derive(Clone, Copy, Debug)]
pub struct Material {
    pub ambient: [Fx; 4],
    pub diffuse: [Fx; 4],
    pub specular: [Fx; 4],
    pub emission: [Fx; 4],
    pub shininess: Fx,
}

impl Default for Material {
    fn default() -> Self {
        let g = |v: Fx| [v, v, v, ONE];
        Material {
            ambient: g(13107),
            diffuse: g(52429),
            specular: g(0),
            emission: g(0),
            shininess: 0,
        }
    }
}

/// The dot product of two vectors of three, rounded once.
pub fn dot3(a: &[Fx; 3], b: &[Fx; 3]) -> Fx {
    narrow((0..3).map(|i| a[i] as i64 * b[i] as i64).sum())
}

/// A vector of three's length, in 16.16.
pub fn length(v: &[Fx; 3]) -> Fx {
    let sq: u64 = v.iter().map(|&c| (c as i64 * c as i64) as u64).sum();
    isqrt(sq) as Fx
}

/// A vector of three made of unit length, or left as it is when it has
/// none.
pub fn normalize(v: &[Fx; 3]) -> [Fx; 3] {
    let len = length(v);
    if len == 0 {
        return *v;
    }
    v.map(|c| div(c, len))
}

/// The colour at a vertex with eye-space position `v` and normal `n`,
/// lit by the lights that are on, in 16.16 with each channel clamped to
/// nought to one.
pub fn shade(
    n: &[Fx; 3],
    v: &[Fx; 4],
    m: &Material,
    lights: &[Light],
    scene: &[Fx; 4],
) -> [Fx; 4] {
    let mut sum = [0i64; 3];
    for (i, s) in sum.iter_mut().enumerate() {
        *s = m.emission[i] as i64 + mul(scene[i], m.ambient[i]) as i64;
    }
    // The vertex in three dimensions, its w divided out.
    let at: [Fx; 3] = if v[3] == ONE || v[3] == 0 {
        [v[0], v[1], v[2]]
    } else {
        core::array::from_fn(|i| div(v[i], v[3]))
    };
    for l in lights.iter().filter(|l| l.on) {
        // The direction to the light, and the attenuation.
        let (to, att) = if l.position[3] == 0 {
            (
                normalize(&[l.position[0], l.position[1], l.position[2]]),
                ONE,
            )
        } else {
            let p: [Fx; 3] =
                core::array::from_fn(|i| div(l.position[i], l.position[3]));
            let vp: [Fx; 3] =
                core::array::from_fn(|i| p[i].saturating_sub(at[i]));
            let d = length(&vp);
            let [k0, k1, k2] = l.attenuation;
            let att = if k1 == 0 && k2 == 0 {
                div(ONE, k0)
            } else {
                div(
                    ONE,
                    k0.saturating_add(mul(k1, d))
                        .saturating_add(mul(k2, mul(d, d))),
                )
            };
            (normalize(&vp), att)
        };
        let spot = if l.spot_cutoff == 180 * ONE {
            ONE
        } else {
            let away = [-to[0], -to[1], -to[2]];
            let s = dot3(&away, &normalize(&l.spot_direction));
            if s < sin_cos(l.spot_cutoff).1 {
                0
            } else {
                pow(s.max(0), l.spot_exponent)
            }
        };
        let k = mul(att, spot);
        if k == 0 {
            continue;
        }
        let nl = dot3(n, &to).max(0);
        let shine = if nl > 0 {
            let h = normalize(&[to[0], to[1], to[2].saturating_add(ONE)]);
            pow(dot3(n, &h).clamp(0, ONE), m.shininess)
        } else {
            0
        };
        for (i, s) in sum.iter_mut().enumerate() {
            let ambient = mul(l.ambient[i], m.ambient[i]) as i64;
            let diffuse = mul(nl, mul(l.diffuse[i], m.diffuse[i])) as i64;
            let specular = mul(shine, mul(l.specular[i], m.specular[i])) as i64;
            *s +=
                mul(k, crate::fixed::sat(ambient + diffuse + specular)) as i64;
        }
    }
    let clamp = |x: i64| x.clamp(0, ONE as i64) as Fx;
    [
        clamp(sum[0]),
        clamp(sum[1]),
        clamp(sum[2]),
        clamp(m.diffuse[3] as i64),
    ]
}
