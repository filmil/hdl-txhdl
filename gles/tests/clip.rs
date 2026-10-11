// SPDX-License-Identifier: Apache-2.0
//! Triangles that cross the near plane (#1661): whatever clipping makes
//! of them reaches Razboj inside its range, never clamped to its edge.
//!
//! `Gl::triangle` decides from a triangle's three corners whether to
//! clip it against the guard band, and then clips it against the near
//! and far planes. Every plane is linear in clip coordinates, and a
//! vertex near-plane clipping makes lies between two corners, so it
//! keeps whatever side of the band both corners were on. What could
//! still put it outside the band is the rounding of its clip
//! coordinates, which is larger in the window the nearer the near
//! plane is; the band keeps a pixel inside Razboj's range for that. A
//! vertex past Razboj's range is clamped to its edge, so a coordinate
//! at exactly `VMIN` or `VMAX` is one that was clamped.
use gles::emit::{VMAX, VMIN};
use gles::fixed::{Fx, ONE};
use gles::{gl, Gl, Vertex};
use razboj::dl::decode;
use razboj::op::Kind;

/// A small generator of the same numbers every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A number in `lo..hi`, in 16.16.
    fn fx(&mut self, lo: f64, hi: f64) -> Fx {
        let u = (self.next() >> 11) as f64 / (1u64 << 53) as f64;
        ((lo + u * (hi - lo)) * ONE as f64) as Fx
    }
}

/// Triangles drawn, and how many reached Razboj with a coordinate
/// clamped to its range, for a frustum whose near plane is `near`.
fn clamped(near: f64, seed: u64, n: usize) -> (usize, usize) {
    let mut rng = Rng(seed);
    let mut frame = vec![[0u32; 16]; 8];
    let (mut drawn, mut clamped) = (0, 0);
    for _ in 0..n {
        let mut g = Gl::new(&mut frame, 640, 480);
        let nf = (near * ONE as f64) as Fx;
        g.matrix_mode(gl::PROJECTION);
        g.frustum(-nf / 2, nf / 2, -3 * nf / 8, 3 * nf / 8, nf, 100 * ONE);
        g.matrix_mode(gl::MODELVIEW);
        // One corner behind the near plane, and two in front of it,
        // wide, so that the crossings land anywhere, the band's edges
        // among them.
        let behind = [
            rng.fx(-4.0, 4.0),
            rng.fx(-4.0, 4.0),
            rng.fx(-near * 0.9, 2.0),
            ONE,
        ];
        let mut front = || {
            let z = rng.fx(-8.0, -near * 1.01);
            let s = -z as f64 / ONE as f64;
            [rng.fx(-2.0 * s, 2.0 * s), rng.fx(-2.0 * s, 2.0 * s), z, ONE]
        };
        let corners = [behind, front(), front()];
        g.draw_vertices(gl::TRIANGLES, 3, |k| Vertex {
            position: corners[k],
            colour: None,
            normal: None,
            tex: None,
        });
        for w in g.frame() {
            let op = decode(w);
            if op.kind != Kind::Tri && op.kind != Kind::Shaded {
                continue;
            }
            drawn += 1;
            let edge = |v: u32| {
                let v = v as u16 as i16 as i32;
                v == VMIN || v == VMAX
            };
            if [op.ax, op.ay, op.bx, op.by, op.cx, op.cy]
                .iter()
                .any(|c| edge(c.raw() as u32))
            {
                clamped += 1;
            }
        }
    }
    (drawn, clamped)
}

/// The near planes the search runs: one, a tenth, and on to a ten
/// thousandth. Before #1661's fix the thousandth clamped 49 of its
/// 24,066 triangles and the ten thousandth more.
const NEARS: [f64; 5] = [1.0, 0.1, 0.01, 0.001, 0.0001];

#[test]
fn no_clipped_vertex_reaches_razboj_clamped() {
    for near in NEARS {
        let (drawn, clamped) = clamped(near, 0x9e37_79b9_7f4a_7c15, 20000);
        assert!(drawn > 20000, "near {near}: {drawn} triangles drawn");
        assert_eq!(clamped, 0, "near {near}: of {drawn} triangles");
    }
}
