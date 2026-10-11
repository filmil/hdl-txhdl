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
            point_size: None,
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

/// A shaded triangle's colour at a window point `(x, y)` in sixteenths,
/// Razboj's rows downwards, read off its planes, in byte units.
fn plane_colour(op: &razboj::op::Insn, x: f64, y: f64) -> [f64; 3] {
    let fx = (op.x0.raw() as f64) * 16.0 + 8.0;
    let fy = (op.y0.raw() as f64) * 16.0 + 8.0;
    let at = |start: u32, dx: u32, dy: u32| {
        let (s, dx, dy) =
            (start as i32 as f64, dx as i32 as f64, dy as i32 as f64);
        // In the thirty-two bits Razboj's adders wrap in, as a start
        // worked out at a pixel outside the triangle may have wrapped.
        let v = s - 32768.0 + (dx * (x - fx) + dy * (y - fy)) / 16.0;
        let m = v.rem_euclid(4294967296.0);
        let m = if m >= 2147483648.0 {
            m - 4294967296.0
        } else {
            m
        };
        m / 65536.0
    };
    [
        at(op.r0.raw() as u32, op.rdx.raw() as u32, op.rdy.raw() as u32),
        at(op.g0.raw() as u32, op.gdx.raw() as u32, op.gdy.raw() as u32),
        at(op.b0.raw() as u32, op.bdx.raw() as u32, op.bdy.raw() as u32),
    ]
}

/// The colours a vertex clipping makes takes are the corners' at the
/// point it cuts, interpolated in clip space (#1661). Smooth triangles
/// with a colour at each corner, cut by the near plane and the guard
/// band in every combination, are drawn; every vertex of every
/// triangle sent to Razboj has the colour its planes give it held to the
/// original triangle's colour at that window point, worked out in f64
/// from the corners' clip coordinates. A cut that put the colours at the
/// wrong place along the edge was out by many levels.
#[test]
fn clipped_colours_are_the_corners_at_the_cut() {
    let (checked, worst) = colours_at_cuts(0x2545_f491_4f6c_dd1d, false);
    assert!(checked > 4000, "{checked} vertices checked");
    eprintln!(
        "{checked} vertices checked, the worst {worst:.3} of a level out"
    );
}

/// The same with the user's clip plane on as well, at a slant through
/// each triangle: its distances are read from the eye, which is not
/// scaled as a kept point is, so its cuts take the colours by another
/// path.
#[test]
fn colours_at_the_user_planes_cut_are_the_corners() {
    let (checked, worst) = colours_at_cuts(0x6a09_e667_f3bc_c908, true);
    assert!(checked > 4000, "{checked} vertices checked");
    eprintln!(
        "{checked} vertices checked, the worst {worst:.3} of a level out"
    );
}

/// Vertices checked and the worst error, in levels, over 4,000 smooth
/// triangles at a near plane of a hundredth, the user's clip plane on
/// through each when `plane` is set.
fn colours_at_cuts(seed: u64, plane: bool) -> (usize, f64) {
    let near = 0.01;
    let mut rng = Rng(seed);
    let mut frame = vec![[0u32; 16]; 8];
    let (mut checked, mut worst) = (0usize, 0.0f64);
    for _ in 0..4000 {
        let mut g = Gl::new(&mut frame, 640, 480);
        let nf = (near * ONE as f64) as Fx;
        let (r, t) = (nf / 2, 3 * nf / 8);
        g.matrix_mode(gl::PROJECTION);
        g.frustum(-r, r, -t, t, nf, 100 * ONE);
        g.matrix_mode(gl::MODELVIEW);
        g.shade_model(gl::SMOOTH);
        let corner = |rng: &mut Rng| {
            let z = rng.fx(-6.0, 1.0);
            [rng.fx(-6.0, 6.0), rng.fx(-6.0, 6.0), z, ONE]
        };
        let pos = [corner(&mut rng), corner(&mut rng), corner(&mut rng)];
        let mut eq = [0.0f64; 4];
        if plane {
            // A plane at a random slant through the triangle's centre,
            // in the eye, which the identity model-view leaves as it is.
            let n = [rng.fx(-1.0, 1.0), rng.fx(-1.0, 1.0), rng.fx(-1.0, 1.0)];
            let d: i64 = (0..3)
                .map(|i| {
                    let c: i64 = pos.iter().map(|p| p[i] as i64).sum();
                    -(n[i] as i64 * c / 3) / ONE as i64
                })
                .sum();
            g.clip_plane(gl::CLIP_PLANE0, &[n[0], n[1], n[2], d as Fx]);
            g.enable(gl::CLIP_PLANE0);
            eq = [n[0], n[1], n[2], d as Fx].map(|v| v as f64 / ONE as f64);
        }
        let col: [[Fx; 4]; 3] = core::array::from_fn(|_| {
            [rng.fx(0.0, 1.0), rng.fx(0.0, 1.0), rng.fx(0.0, 1.0), ONE]
        });
        g.draw_vertices(gl::TRIANGLES, 3, |k| Vertex {
            position: pos[k],
            colour: Some(col[k]),
            normal: None,
            tex: None,
            point_size: None,
        });
        // The corners in clip space, from the matrix the library made,
        // and the corners' colours, in f64.
        let f = |v: Fx| v as f64 / ONE as f64;
        let pj = g.projection();
        let clip: [[f64; 4]; 3] = core::array::from_fn(|k| {
            core::array::from_fn(|row| {
                (0..4).map(|c| f(pj[c * 4 + row]) * f(pos[k][c])).sum()
            })
        });
        for w in g.frame() {
            let op = decode(w);
            if op.kind != Kind::Shaded {
                continue;
            }
            let s = |v: u32| v as u16 as i16 as f64;
            for (vx, vy) in [
                (s(op.ax.raw() as u32), s(op.ay.raw() as u32)),
                (s(op.bx.raw() as u32), s(op.by.raw() as u32)),
                (s(op.cx.raw() as u32), s(op.cy.raw() as u32)),
            ] {
                // The original triangle's colour at a window point, from
                // the clip-space barycentrics of the point of it that lands
                // there, and that point's distance from the user's plane;
                // `None` where they cannot be told.
                let colour_at = |x: f64, y: f64| -> Option<([f64; 3], f64)> {
                    let nx = (x / 16.0 - 320.0) / 320.0;
                    let ny = ((480.0 * 16.0 - y) / 16.0 - 240.0) / 240.0;
                    let m: [[f64; 3]; 3] = [
                        core::array::from_fn(|k| clip[k][0] - nx * clip[k][3]),
                        core::array::from_fn(|k| clip[k][1] - ny * clip[k][3]),
                        [1.0, 1.0, 1.0],
                    ];
                    let det = |m: &[[f64; 3]; 3]| {
                        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
                            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
                            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
                    };
                    let d = det(&m);
                    if d.abs() < 1e-12 {
                        return None;
                    }
                    let lam: [f64; 3] = core::array::from_fn(|k| {
                        let mut mk = m;
                        for (row, val) in [0.0, 0.0, 1.0].iter().enumerate() {
                            mk[row][k] = *val;
                        }
                        det(&mk) / d
                    });
                    // A point behind the eye lands there too; it is not
                    // the triangle's.
                    if (0..3).map(|k| lam[k] * clip[k][3]).sum::<f64>() <= 0.0 {
                        return None;
                    }
                    let side: f64 = (0..3)
                        .map(|k| {
                            lam[k]
                                * (0..4)
                                    .map(|c| eq[c] * f(pos[k][c]))
                                    .sum::<f64>()
                        })
                        .sum();
                    let colour = core::array::from_fn(|ch| {
                        255.0
                            * (0..3)
                                .map(|k| lam[k] * f(col[k][ch]))
                                .sum::<f64>()
                    });
                    Some((colour, side))
                };
                // The vertex's true place is within half a sixteenth of
                // where it was rounded to, so its colour is held to the
                // range the colour spans over that square, give or take
                // a level and a half for the colours' own rounding.
                let mut lo = [f64::MAX; 3];
                let mut hi = [f64::MIN; 3];
                let mut side = f64::MIN;
                for (ox, oy) in [
                    (-0.5, -0.5),
                    (0.5, -0.5),
                    (-0.5, 0.5),
                    (0.5, 0.5),
                    (0.0, 0.0),
                ] {
                    let Some((c, d)) = colour_at(vx + ox, vy + oy) else {
                        continue;
                    };
                    side = side.max(d);
                    for ch in 0..3 {
                        lo[ch] = lo[ch].min(c[ch]);
                        hi[ch] = hi[ch].max(c[ch]);
                    }
                }
                if lo[0] == f64::MAX {
                    continue;
                }
                // Nothing is drawn past the user's plane: the distance is
                // projectively linear over the square, so some corner of
                // it is on the kept side if the true place is. The plane
                // reads the eye, which a near plane's cut rounds to 16.16,
                // so a cut on it may be off it by half a unit in each.
                let tol = eq[..3].iter().map(|v| v.abs()).sum::<f64>()
                    / (1u64 << 17) as f64
                    + 1e-9;
                assert!(
                    side >= -tol,
                    "a vertex at ({vx}, {vy}) is {side} past the user's plane"
                );
                let got = plane_colour(&op, vx, vy);
                for ch in 0..3 {
                    let (l, u) =
                        (lo[ch].max(0.0) - 1.5, hi[ch].min(255.0) + 1.5);
                    let e = if got[ch] < l {
                        l - got[ch]
                    } else if got[ch] > u {
                        got[ch] - u
                    } else {
                        0.0
                    };
                    worst = worst.max(e);
                    assert!(
                        e == 0.0,
                        "a vertex at ({vx}, {vy}): channel {ch} is {:.2}, wants {:.2} to {:.2}",
                        got[ch],
                        lo[ch],
                        hi[ch]
                    );
                }
                checked += 1;
            }
        }
    }
    (checked, worst)
}
