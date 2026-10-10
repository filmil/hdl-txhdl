// SPDX-License-Identifier: Apache-2.0
//! The canonical teapot (#1592): Newell's Utah teapot, from the control
//! points `//lib/teapot` holds, cut into triangles once at the start and
//! drawn through GL as the icosahedron is, smooth shaded.
//!
//! The ten input patches become Newell's 32: the rim, the body's two,
//! the lid's two and the bottom are each a quarter of a surface of
//! revolution, turned to the other three quarters, and the handle's two
//! and the spout's two are halves, mirrored across the plane through the
//! axis. Each patch is cut into `N` by `N` squares, two triangles each,
//! over a grid of `(N + 1)` by `(N + 1)` points the squares share. A
//! square that a row of the patch closes to a point, at the bottom's
//! middle, is one triangle and not two; the lid's knob closes its first
//! row nearly, to within 0.002, and keeps both.
//!
//! A point's normal is the cross of the patch's two slopes there, as
//! freeglut works it out, and on the rows that close to a point, where
//! both slopes vanish, straight up at the knob and straight down under
//! the bottom. The teapot is laid with its axis along y, its middle at
//! the origin and `SCALE` of the data's size, as freeglut lays it, so
//! that it turns about its own axis in front of the eye.
//!
//! The arithmetic is in `f64`, which the core has no unit for and works
//! in software; it runs once, before the first frame.
//!
//! The tessellation, the view, the light and the frames are the canonical
//! teapot's and do not change: a milestone draws the same teapot again,
//! and its frame rate is the measure. A finer teapot is another demo.
use core::sync::atomic::{AtomicBool, Ordering};
use gles::fixed::{Fx, ONE};
use teapot_data::{PATCHES, POINTS};

/// Squares a patch's side is cut into.
pub const N: usize = 3;
/// Newell's patches: six input patches turned to four quarters, and
/// four mirrored to two halves.
pub const COUNT: usize = 6 * 4 + 4 * 2;
/// The points, and the most triangles, the cut gives.
pub const VERTS: usize = COUNT * (N + 1) * (N + 1);
pub const TRIANGLES: usize = COUNT * 2 * N * N;

/// The data's teapot is 3.15 high, its base at nought, and about 6.5
/// from the spout's tip to the handle's back.
const MIDDLE: f64 = 1.575;
const SCALE: f64 = 0.55;

/// A triangle smaller than this, in square units, is a point.
const SPECK: f64 = 1e-9;

/// The teapot as GL takes it: positions and normals in 16.16, and the
/// indices of `triangles` triangles.
pub struct Teapot {
    pub positions: [[Fx; 4]; VERTS],
    pub normals: [[Fx; 3]; VERTS],
    pub indices: [u16; 3 * TRIANGLES],
    pub triangles: usize,
}

impl Teapot {
    /// Nothing cut yet, for a static.
    pub const EMPTY: Teapot = Teapot {
        positions: [[0; 4]; VERTS],
        normals: [[0; 3]; VERTS],
        indices: [0; 3 * TRIANGLES],
        triangles: 0,
    };

    /// Cut the teapot into triangles, in place, since it is too large
    /// for a stack.
    pub fn build(&mut self) {
        let mut out = 0;
        self.triangles = 0;
        for (p, patch) in PATCHES.iter().enumerate() {
            let cp = control(patch);
            let (pos, nor) = grid(&cp, p);
            let copies = if p < 6 { 4 } else { 2 };
            for copy in 0..copies {
                let base = out * (N + 1) * (N + 1);
                for r in 0..=N {
                    for c in 0..=N {
                        // A mirrored copy reverses its rows, so that its
                        // triangles turn the same way as the original's.
                        let from = if copies == 2 && copy == 1 {
                            (N - r) * (N + 1) + c
                        } else {
                            r * (N + 1) + c
                        };
                        let at = base + r * (N + 1) + c;
                        let q = place(pos[from], copies, copy);
                        let n = place(nor[from], copies, copy);
                        self.positions[at] =
                            [fx(q[0]), fx(q[1]), fx(q[2]), ONE];
                        self.normals[at] = [fx(n[0]), fx(n[1]), fx(n[2])];
                    }
                }
                for r in 0..N {
                    for c in 0..N {
                        let k = |r: usize, c: usize| base + r * (N + 1) + c;
                        let (a, b, d, e) = (
                            k(r, c),
                            k(r + 1, c),
                            k(r, c + 1),
                            k(r + 1, c + 1),
                        );
                        for t in [[a, b, d], [b, e, d]] {
                            if self.area(t) > SPECK {
                                let at = 3 * self.triangles;
                                for (i, &v) in t.iter().enumerate() {
                                    self.indices[at + i] = v as u16;
                                }
                                self.triangles += 1;
                            }
                        }
                    }
                }
                out += 1;
            }
        }
    }

    /// The square of twice a triangle's area, in the data's units.
    fn area(&self, t: [usize; 3]) -> f64 {
        let p = |i: usize| {
            let v = self.positions[t[i]];
            [v[0] as f64, v[1] as f64, v[2] as f64].map(|x| x / ONE as f64)
        };
        let (a, b, c) = (p(0), p(1), p(2));
        let n = cross(sub(b, a), sub(c, a));
        dot(n, n)
    }
}

/// A patch's sixteen control points, laid with the axis along y, the
/// middle at the origin and `SCALE` of the size: `(x, z - MIDDLE, -y)`.
fn control(patch: &[u8; 16]) -> [[[f64; 3]; 4]; 4] {
    let mut cp = [[[0.0; 3]; 4]; 4];
    for (i, &k) in patch.iter().enumerate() {
        let [x, y, z] = POINTS[k as usize].map(|v| v as f64);
        cp[i / 4][i % 4] = [x * SCALE, (z - MIDDLE) * SCALE, -y * SCALE];
    }
    cp
}

/// The cubic Bernstein polynomials at `t`, and their slopes.
fn bernstein(t: f64) -> ([f64; 4], [f64; 4]) {
    let s = 1.0 - t;
    (
        [s * s * s, 3.0 * t * s * s, 3.0 * t * t * s, t * t * t],
        [
            -3.0 * s * s,
            3.0 * s * s - 6.0 * t * s,
            6.0 * t * s - 3.0 * t * t,
            3.0 * t * t,
        ],
    )
}

/// The patch's points and unit normals on the grid, rows along the
/// first index of `cp`. The input patches 3 and 5, the lid's knob and
/// the bottom, close their first row to a point, whose normal is up and
/// down.
type Grid = [[f64; 3]; (N + 1) * (N + 1)];
fn grid(cp: &[[[f64; 3]; 4]; 4], patch: usize) -> (Grid, Grid) {
    let (mut pos, mut nor) =
        ([[0.0; 3]; (N + 1) * (N + 1)], [[0.0; 3]; (N + 1) * (N + 1)]);
    for r in 0..=N {
        let (bu, du) = bernstein(r as f64 / N as f64);
        for c in 0..=N {
            let (bv, dv) = bernstein(c as f64 / N as f64);
            let (mut p, mut tv, mut tu) = ([0.0; 3], [0.0; 3], [0.0; 3]);
            for i in 0..4 {
                for j in 0..4 {
                    for k in 0..3 {
                        p[k] += bu[i] * bv[j] * cp[i][j][k];
                        tv[k] += bu[i] * dv[j] * cp[i][j][k];
                        tu[k] += du[i] * bv[j] * cp[i][j][k];
                    }
                }
            }
            let at = r * (N + 1) + c;
            pos[at] = p;
            nor[at] = match (patch, r) {
                (3, 0) => [0.0, 1.0, 0.0],
                (5, 0) => [0.0, -1.0, 0.0],
                _ => unit(cross(tv, tu)),
            };
        }
    }
    (pos, nor)
}

/// A point or a normal in its copy: one of the four quarters, turned
/// about y, or one of the two halves, mirrored across z = 0.
fn place([x, y, z]: [f64; 3], copies: usize, copy: usize) -> [f64; 3] {
    match (copies, copy) {
        (4, 1) => [z, y, -x],
        (4, 2) => [-x, y, -z],
        (4, 3) => [-z, y, x],
        (2, 1) => [x, y, -z],
        _ => [x, y, z],
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `v` at unit length, by Newton's method, since `core` has no square
/// root; nought stays nought.
fn unit(v: [f64; 3]) -> [f64; 3] {
    let sq = dot(v, v);
    if sq <= 0.0 {
        return v;
    }
    let mut r = if sq > 1.0 { sq } else { 1.0 };
    for _ in 0..64 {
        r = 0.5 * (r + sq / r);
    }
    v.map(|x| x / r)
}

/// A number in 16.16, rounded to the nearest.
fn fx(v: f64) -> Fx {
    let s = v * ONE as f64;
    (if s < 0.0 { s - 0.5 } else { s + 0.5 }) as Fx
}

/// The teapot, cut on the first call (#1592). Hart 0 calls first, for
/// the first frame, before it starts hart 1, so the two never cut it at
/// once; every later call finds it cut.
pub fn get() -> &'static Teapot {
    static mut TEAPOT: Teapot = Teapot::EMPTY;
    static CUT: AtomicBool = AtomicBool::new(false);
    if !CUT.load(Ordering::Acquire) {
        // SAFETY: only the first call writes, before any other reads.
        unsafe { (*core::ptr::addr_of_mut!(TEAPOT)).build() };
        CUT.store(true, Ordering::Release);
    }
    // SAFETY: cut, and never written again.
    unsafe { &*core::ptr::addr_of!(TEAPOT) }
}
