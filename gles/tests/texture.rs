// SPDX-License-Identifier: Apache-2.0
//! Textures through the GL library (#997), against floating point,
//! through Razboj's model.
//!
//! A texture uploaded with its levels generated, on a quad in
//! perspective, under GL's filters and environments: the library's frame,
//! drawn by Razboj's model as a tile table draws it, is held pixel by
//! pixel to a reference that runs GL's pipeline in `f64` from the image
//! itself: the projection, the viewport, `s/w`, `t/w` and `q/w` across
//! the window, the scale factor GL allows, the levels as the mean of the
//! two by two above, the filter, and the environment. A pixel is left out
//! where the answer turns on less than the arithmetic resolves.
use gles::fixed::{Fx, ONE};
use gles::{gl, Gl, Vertex};
use razboj::dl::decode_list;
use razboj::model::{render_textured, Textures};
use razboj_tile::WORDS;

const W: u32 = 128;
const H: u32 = 96;
const BUS: u32 = 0x0010_0000;
const SIDE: usize = 32;

fn fx(v: f64) -> Fx {
    (v * 65536.0).round() as Fx
}

/// The image: RGBA bytes, a pattern of each texel's place.
fn image() -> Vec<u8> {
    let mut p = Vec::new();
    for j in 0..SIDE {
        for i in 0..SIDE {
            p.extend_from_slice(&[
                (i * 8) as u8,
                (j * 8) as u8,
                ((i ^ j) * 8) as u8,
                (128 + (i + j) * 2) as u8,
            ]);
        }
    }
    p
}

/// The levels as GL defines them from the image: each texel the mean of
/// the two by two above it, rounded, a channel at a time.
fn levels() -> Vec<Vec<[f64; 4]>> {
    let img = image();
    let mut out = vec![(0..SIDE * SIDE)
        .map(|k| {
            let b = &img[4 * k..4 * k + 4];
            [b[0], b[1], b[2], b[3]].map(|v| v as f64)
        })
        .collect::<Vec<_>>()];
    let mut side = SIDE;
    while side > 1 {
        let up = out.last().unwrap();
        let s = side / 2;
        let next = (0..s * s)
            .map(|k| {
                let (i, j) = (k % s, k / s);
                std::array::from_fn(|c| {
                    let t = |x: usize, y: usize| up[y * side + x][c];
                    let sum = t(2 * i, 2 * j)
                        + t(2 * i + 1, 2 * j)
                        + t(2 * i, 2 * j + 1)
                        + t(2 * i + 1, 2 * j + 1);
                    (sum / 4.0).round()
                })
            })
            .collect();
        out.push(next);
        side = s;
    }
    out
}

/// The quad's corners in object coordinates, and their texture
/// coordinates: a floor going away, seen through a frustum.
const CORNERS: [([f64; 3], [f64; 2]); 4] = [
    ([-1.0, -0.6, -1.5], [0.0, 0.0]),
    ([1.0, -0.6, -1.5], [3.0, 0.0]),
    ([1.0, -0.6, -8.0], [3.0, 4.0]),
    ([-1.0, -0.6, -8.0], [0.0, 4.0]),
];

/// The frustum: a near plane a unit away, as wide as the screen's shape.
const FRUSTUM: [f64; 6] = [-0.5, 0.5, -0.375, 0.375, 1.0, 20.0];

/// The library's frame of the quad, textured with `min`, `mag` and `env`,
/// and its room.
fn library(min: u32, mag: u32, env: u32) -> (Vec<[u32; WORDS]>, Vec<u32>) {
    let mut frame = vec![[0u32; WORDS]; 512];
    let room: &'static mut [u32] =
        Box::leak(vec![0u32; 1 << 16].into_boxed_slice());
    let mut g = Gl::new(&mut frame, W, H);
    g.texture_room(room, BUS);
    let mut name = [0u32];
    g.gen_textures(&mut name);
    g.bind_texture(gl::TEXTURE_2D, name[0]);
    g.tex_parameter(gl::TEXTURE_2D, gl::GENERATE_MIPMAP, 1);
    g.tex_parameter(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, min);
    g.tex_parameter(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, mag);
    let img = image();
    let (s, rgba, ub) = (SIDE as u32, gl::RGBA, gl::UNSIGNED_BYTE);
    g.tex_image_2d(gl::TEXTURE_2D, 0, rgba, s, s, 0, rgba, ub, &img);
    g.tex_env(gl::TEXTURE_ENV, gl::TEXTURE_ENV_MODE, &[env as Fx]);
    g.tex_env(
        gl::TEXTURE_ENV,
        gl::TEXTURE_ENV_COLOR,
        &[ONE / 4, ONE, ONE / 2, ONE],
    );
    g.enable(gl::TEXTURE_2D);
    let [l, r, b, t, n, f] = FRUSTUM.map(fx);
    g.matrix_mode(gl::PROJECTION);
    g.frustum(l, r, b, t, n, f);
    g.matrix_mode(gl::MODELVIEW);
    g.color(ONE, ONE / 2, ONE, ONE);
    g.shade_model(gl::FLAT);
    let vertex = |k: usize| {
        let (p, st) = CORNERS[[0, 1, 2, 0, 2, 3][k]];
        Vertex {
            position: [fx(p[0]), fx(p[1]), fx(p[2]), ONE],
            colour: None,
            normal: None,
            tex: Some([fx(st[0]), fx(st[1]), 0, ONE]),
        }
    };
    g.draw_vertices(gl::TRIANGLES, 6, vertex);
    assert_eq!(g.get_error(), gl::NO_ERROR);
    assert!(g.tiled(), "a textured frame is a tile table");
    let used = g.frame().len();
    let words = g.textures().unwrap().words().to_vec();
    (frame[..used].to_vec(), words)
}

/// A point of the quad in the window, from its object coordinates: x and
/// y in pixels, y downwards, and its clip w.
fn window(p: [f64; 3]) -> (f64, f64, f64) {
    let [l, r, b, t, n, f] = FRUSTUM;
    let w = -p[2];
    let xc = 2.0 * n / (r - l) * p[0] + (r + l) / (r - l) * p[2];
    let yc = 2.0 * n / (t - b) * p[1] + (t + b) / (t - b) * p[2];
    let _ = f;
    let (xn, yn) = (xc / w, yc / w);
    (
        W as f64 * (xn + 1.0) / 2.0,
        H as f64 * (1.0 - (yn + 1.0) / 2.0),
        w,
    )
}

/// GL's texture coordinates at a window point of the triangle `k`, in
/// texels: `s/w` and `t/w` over `1/w`, interpolated in the window.
fn texels(k: [usize; 3], at: (f64, f64)) -> Option<(f64, f64)> {
    let v = k.map(|n| (window(CORNERS[n].0), CORNERS[n].1));
    let ((x0, y0, _), _) = v[0];
    let ((x1, y1, _), _) = v[1];
    let ((x2, y2, _), _) = v[2];
    let area = (x1 - x0) * (y2 - y0) - (y1 - y0) * (x2 - x0);
    let l1 = ((at.0 - x0) * (y2 - y0) - (at.1 - y0) * (x2 - x0)) / area;
    let l2 = ((x1 - x0) * (at.1 - y0) - (y1 - y0) * (at.0 - x0)) / area;
    let l = [1.0 - l1 - l2, l1, l2];
    if l.iter().any(|&x| x < 0.0) {
        return None;
    }
    let (mut s, mut t, mut q) = (0.0, 0.0, 0.0);
    for n in 0..3 {
        let ((_, _, w), st) = v[n];
        s += l[n] * st[0] / w;
        t += l[n] * st[1] / w;
        q += l[n] / w;
    }
    Some((s / q * SIDE as f64, t / q * SIDE as f64))
}

/// GL's level of detail at a window point: `log2` of the largest of the
/// four derivatives, the scale factor GL ES allows.
fn lod(k: [usize; 3], at: (f64, f64)) -> f64 {
    let h = 1e-3;
    let (u, v) = texels(k, at).unwrap();
    let (ux, vx) = texels(k, (at.0 + h, at.1)).unwrap_or((u, v));
    let (uy, vy) = texels(k, (at.0, at.1 + h)).unwrap_or((u, v));
    [(ux - u) / h, (vx - v) / h, (uy - u) / h, (vy - v) / h]
        .iter()
        .fold(0f64, |m, d| m.max(d.abs()))
        .log2()
}

/// GL's sampling of the levels, repeated, at `(u, v)` and `λ`.
fn sample(
    lv: &[Vec<[f64; 4]>],
    u: f64,
    v: f64,
    lam: f64,
    min: u32,
    mag: u32,
) -> [f64; 4] {
    let top = (lv.len() - 1) as f64;
    let level = |l: usize, f: u32| {
        let side = SIDE >> l;
        let texel = |i: i64, j: i64| {
            let n = side as i64;
            lv[l][(j.rem_euclid(n) * n + i.rem_euclid(n)) as usize]
        };
        let (u, v) = (u / (1 << l) as f64, v / (1 << l) as f64);
        if f == gl::NEAREST {
            return texel(u.floor() as i64, v.floor() as i64);
        }
        let (u, v) = (u - 0.5, v - 0.5);
        let (i, j) = (u.floor() as i64, v.floor() as i64);
        let (a, b) = (u - u.floor(), v - v.floor());
        let t = [
            texel(i, j),
            texel(i + 1, j),
            texel(i, j + 1),
            texel(i + 1, j + 1),
        ];
        let w = [(1.0 - a) * (1.0 - b), a * (1.0 - b), (1.0 - a) * b, a * b];
        std::array::from_fn(|c| (0..4).map(|k| w[k] * t[k][c]).sum())
    };
    let c = if mag == gl::LINEAR
        && (min == gl::NEAREST_MIPMAP_NEAREST
            || min == gl::NEAREST_MIPMAP_LINEAR)
    {
        0.5
    } else {
        0.0
    };
    if lam <= c {
        return level(0, mag);
    }
    let (f, mips) = match min {
        gl::NEAREST | gl::LINEAR => return level(0, min),
        gl::NEAREST_MIPMAP_NEAREST => (gl::NEAREST, false),
        gl::LINEAR_MIPMAP_NEAREST => (gl::LINEAR, false),
        gl::NEAREST_MIPMAP_LINEAR => (gl::NEAREST, true),
        _ => (gl::LINEAR, true),
    };
    if !mips {
        let l = if lam <= 0.5 {
            0.0
        } else {
            ((lam + 0.5).ceil() - 1.0).min(top)
        };
        return level(l as usize, f);
    }
    if lam >= top {
        return level(top as usize, f);
    }
    let l = lam.floor();
    let (t1, t2) = (level(l as usize, f), level(l as usize + 1, f));
    std::array::from_fn(|c| (1.0 - (lam - l)) * t1[c] + (lam - l) * t2[c])
}

/// GL's environment on an RGBA texel, channels red first.
fn env(mode: u32, f: [f64; 4], t: [f64; 4], e: [f64; 4]) -> [f64; 4] {
    let m = |a: f64, b: f64| a * b / 255.0;
    std::array::from_fn(|c| match (mode, c) {
        (gl::REPLACE, _) => t[c],
        (gl::MODULATE, _) => m(f[c], t[c]),
        (gl::DECAL, 3) => f[3],
        (gl::DECAL, _) => (f[c] * (255.0 - t[3]) + t[c] * t[3]) / 255.0,
        (gl::BLEND, 3) => m(f[3], t[3]),
        (gl::BLEND, _) => (f[c] * (255.0 - t[c]) + e[c] * t[c]) / 255.0,
        (_, 3) => m(f[3], t[3]),
        _ => (f[c] + t[c]).min(255.0),
    })
}

/// Every filter and every environment: each pixel of the picture is GL's
/// within 3 a channel, at the model's own level of detail, which is GL's
/// within a twentieth of a level, of what GL gives within the sixteenth
/// of a pixel a snapped vertex moves; pixels are left out near the quad's
/// edges and near a level's edge.
#[test]
fn textures_are_floating_points() {
    let lv = levels();
    let filters = [
        (gl::NEAREST, gl::NEAREST),
        (gl::LINEAR, gl::LINEAR),
        (gl::NEAREST_MIPMAP_NEAREST, gl::LINEAR),
        (gl::LINEAR_MIPMAP_NEAREST, gl::NEAREST),
        (gl::NEAREST_MIPMAP_LINEAR, gl::NEAREST),
        (gl::LINEAR_MIPMAP_LINEAR, gl::LINEAR),
    ];
    let envs = [gl::REPLACE, gl::MODULATE, gl::DECAL, gl::BLEND, gl::ADD];
    let (frag, ecol) =
        ([255.0, 128.0, 255.0, 255.0], [64.0, 255.0, 128.0, 255.0]);
    let (mut compared, mut worst) = (0, 0f64);
    for &(min, mag) in &filters {
        for &e in &envs {
            let (frame, words) = library(min, mag, e);
            let list = decode_list(&frame);
            let read = |a: u32| words[((a - BUS) / 4) as usize];
            let t = Textures { mem: &read };
            let (w, h) = (W as usize, H as usize);
            let got = render_textured(&list, w, h, vec![0; w * h], Some(&t));
            for y in 0..h {
                for x in 0..w {
                    let at = (x as f64 + 0.5, y as f64 + 0.5);
                    let Some((k, (u, v))) = [[0, 1, 2], [0, 2, 3]]
                        .into_iter()
                        .find_map(|k| Some((k, texels(k, at)?)))
                    else {
                        continue;
                    };
                    // The quad's own edges, and the clipped polygon's.
                    let edge =
                        [(0.5, 0.0), (-0.5, 0.0), (0.0, 0.5), (0.0, -0.5)]
                            .iter()
                            .any(|d| {
                                texels(k, (at.0 + d.0 * 3.0, at.1 + d.1 * 3.0))
                                    .is_none()
                            });
                    let inside_other = [[0, 1, 2], [0, 2, 3]]
                        .iter()
                        .filter(|&&kk| kk != k)
                        .any(|&kk| texels(kk, at).is_some());
                    if edge || inside_other {
                        continue;
                    }
                    let lam = lod(k, at);
                    // The model's level of detail at this pixel, from the
                    // entry that drew it.
                    let Some(ml) = list.iter().find_map(|i| model_lod(i, x, y))
                    else {
                        continue;
                    };
                    worst = worst.max((ml - lam).abs());
                    let near = |x: f64, off: f64| {
                        ((x - off) - (x - off).round()).abs()
                    };
                    // A level's edge, where the filter changes.
                    if near(ml, 0.5) < 0.03 || near(ml, 0.0) < 0.03 {
                        continue;
                    }
                    // A vertex snapped to a sixteenth of a pixel moves the
                    // texels by a sixteenth of what a pixel spans, so the
                    // pixel is held to what GL gives anywhere that far
                    // away: wide where the image is steep, such as across
                    // a texel's edge under the nearest filters, or across
                    // the seam where the image repeats.
                    let s = 0.02 + ml.exp2() / 16.0;
                    let mut lo = [f64::MAX; 4];
                    let mut hi = [f64::MIN; 4];
                    for (du, dv) in
                        [(0.0, 0.0), (-s, -s), (-s, s), (s, -s), (s, s)]
                    {
                        let texel = sample(&lv, u + du, v + dv, ml, min, mag);
                        let want = env(e, frag, texel, ecol);
                        for c in 0..4 {
                            lo[c] = lo[c].min(want[c]);
                            hi[c] = hi[c].max(want[c]);
                        }
                    }
                    let p = got[y * w + x];
                    let have = [16, 8, 0, 24].map(|s| ((p >> s) & 0xff) as f64);
                    for c in 0..4 {
                        assert!(
                            have[c] >= lo[c] - 3.0 && have[c] <= hi[c] + 3.0,
                            "min {min:04x} mag {mag:04x} env {e:04x} ({x}, {y}) \
                             channel {c}: {} not in {:.2}..{:.2}; \
                             u {u:.3} v {v:.3} λ {ml:.3}",
                            have[c],
                            lo[c],
                            hi[c]
                        );
                    }
                    compared += 1;
                }
            }
        }
    }
    println!("{compared} compared, λ off by {worst:.3} at most");
    assert!(compared > 30 * 1000, "{compared} compared");
    assert!(worst < 0.05, "λ off by {worst:.3}");
}

/// The model's level of detail at the pixel `(x, y)`, in levels, from the
/// textured entry whose box holds it.
fn model_lod(i: &razboj::op::Insn, x: usize, y: usize) -> Option<f64> {
    if !i.tex.to_bool() {
        return None;
    }
    let (x0, y0) = (i.x0.raw() as i32, i.y0.raw() as i32);
    let (x1, y1) = (i.x1.raw() as i32, i.y1.raw() as i32);
    let (x, y) = (x as i32, y as i32);
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return None;
    }
    let (dx, dy) = (x - x0, y - y0);
    let q = (i.q0.raw() as u64)
        .wrapping_add((i.qdx.raw() as u64).wrapping_mul(dx as u64))
        .wrapping_add((i.qdy.raw() as u64).wrapping_mul(dy as u64));
    let n = razboj::tex::numerators(i, dx, dy);
    Some(razboj::tex::lod(n, i.lodk.raw() as u32, q) as f64 / 256.0)
}

/// The formats GL ES uploads, each as the library stores it: RGBA in 32
/// bits, the channels it lacks filled as GL fills them.
#[test]
fn the_formats_upload_as_gl_says() {
    let mut frame = vec![[0u32; WORDS]; 4];
    let room: &'static mut [u32] =
        Box::leak(vec![0u32; 1 << 14].into_boxed_slice());
    let mut g = Gl::new(&mut frame, W, H);
    g.texture_room(room, BUS);
    let mut name = [0u32];
    g.gen_textures(&mut name);
    g.bind_texture(gl::TEXTURE_2D, name[0]);
    g.tex_parameter(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST);
    let cases: [(u32, u32, Vec<u8>, u32); 7] = [
        (gl::RGBA, gl::UNSIGNED_BYTE, vec![1, 2, 3, 4], 0x0401_0203),
        (gl::RGB, gl::UNSIGNED_BYTE, vec![1, 2, 3], 0xff01_0203),
        (gl::LUMINANCE, gl::UNSIGNED_BYTE, vec![9], 0xff09_0909),
        (
            gl::LUMINANCE_ALPHA,
            gl::UNSIGNED_BYTE,
            vec![9, 7],
            0x0709_0909,
        ),
        (gl::ALPHA, gl::UNSIGNED_BYTE, vec![7], 0x0700_0000),
        (
            gl::RGB,
            gl::UNSIGNED_SHORT_5_6_5,
            vec![0x1f, 0xf8],
            0xffff_00ff,
        ),
        (
            gl::RGBA,
            gl::UNSIGNED_SHORT_4_4_4_4,
            vec![0x34, 0x12],
            0x4411_2233,
        ),
    ];
    for (format, ty, px, want) in cases {
        g.tex_image_2d(gl::TEXTURE_2D, 0, format, 1, 1, 0, format, ty, &px);
        assert_eq!(g.get_error(), gl::NO_ERROR, "{format:04x} {ty:04x}");
        let d = g.textures().unwrap().desc(name[0]).unwrap();
        let at = ((d.base - BUS) / 4) as usize;
        let got = g.textures().unwrap().words()[at];
        assert_eq!(got, want, "{format:04x} {ty:04x}: {got:08x}");
    }
    // A side not a power of two, and a border, are GL_INVALID_VALUE.
    let px = [0u8; 12];
    g.tex_image_2d(
        gl::TEXTURE_2D,
        0,
        gl::RGBA,
        3,
        1,
        0,
        gl::RGBA,
        gl::UNSIGNED_BYTE,
        &px,
    );
    assert_eq!(g.get_error(), gl::INVALID_VALUE);
    g.tex_image_2d(
        gl::TEXTURE_2D,
        0,
        gl::RGBA,
        1,
        1,
        1,
        gl::RGBA,
        gl::UNSIGNED_BYTE,
        &px,
    );
    assert_eq!(g.get_error(), gl::INVALID_VALUE);
}

/// The paletted formats of `OES_compressed_paletted_texture` (#998):
/// each of the ten, with a palette of random entries and random indices
/// over a texture eight by four and its three levels below, uploads as
/// GL says: each texel its index's entry, a channel of `n` bits taken
/// as `c / (2^n - 1)` of 255, within one; a format of three channels
/// fills alpha with 255. A positive level and data short of the
/// levels it names are `GL_INVALID_VALUE`.
#[test]
fn the_paletted_formats_upload_as_gl_says() {
    use razboj_tile::tex::texel_offset;
    let mut frame = vec![[0u32; WORDS]; 4];
    let room: &'static mut [u32] =
        Box::leak(vec![0u32; 1 << 14].into_boxed_slice());
    let mut g = Gl::new(&mut frame, W, H);
    g.texture_room(room, BUS);
    let mut name = [0u32];
    g.gen_textures(&mut name);
    g.bind_texture(gl::TEXTURE_2D, name[0]);
    let mut x = 0x9a1e_77e5u32;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    // Each format: its bits an index, the bytes an entry, and an entry's
    // channels from its bytes, each a value and its bits.
    type Entry = fn(&[u8]) -> [(u32, u32); 4];
    let rgb8: Entry = |b| {
        [
            (b[0] as u32, 8),
            (b[1] as u32, 8),
            (b[2] as u32, 8),
            (255, 8),
        ]
    };
    let rgba8: Entry = |b| {
        [
            (b[0] as u32, 8),
            (b[1] as u32, 8),
            (b[2] as u32, 8),
            (b[3] as u32, 8),
        ]
    };
    let r565: Entry = |b| {
        let s = b[0] as u32 | (b[1] as u32) << 8;
        [(s >> 11, 5), ((s >> 5) & 63, 6), (s & 31, 5), (255, 8)]
    };
    let rgba4: Entry = |b| {
        let s = b[0] as u32 | (b[1] as u32) << 8;
        [
            (s >> 12, 4),
            ((s >> 8) & 15, 4),
            ((s >> 4) & 15, 4),
            (s & 15, 4),
        ]
    };
    let r5a1: Entry = |b| {
        let s = b[0] as u32 | (b[1] as u32) << 8;
        [
            (s >> 11, 5),
            ((s >> 6) & 31, 5),
            ((s >> 1) & 31, 5),
            (s & 1, 1),
        ]
    };
    let formats: [(u32, u32, usize, Entry); 10] = [
        (gl::PALETTE4_RGB8_OES, 4, 3, rgb8),
        (gl::PALETTE4_RGBA8_OES, 4, 4, rgba8),
        (gl::PALETTE4_R5_G6_B5_OES, 4, 2, r565),
        (gl::PALETTE4_RGBA4_OES, 4, 2, rgba4),
        (gl::PALETTE4_RGB5_A1_OES, 4, 2, r5a1),
        (gl::PALETTE8_RGB8_OES, 8, 3, rgb8),
        (gl::PALETTE8_RGBA8_OES, 8, 4, rgba8),
        (gl::PALETTE8_R5_G6_B5_OES, 8, 2, r565),
        (gl::PALETTE8_RGBA4_OES, 8, 2, rgba4),
        (gl::PALETTE8_RGB5_A1_OES, 8, 2, r5a1),
    ];
    let sides = [(8u32, 4u32), (4, 2), (2, 1), (1, 1)];
    for (format, bits, entry, channels) in formats {
        let entries = 1usize << bits;
        let mut data: Vec<u8> =
            (0..entries * entry).map(|_| next() as u8).collect();
        let mut indices = Vec::new();
        for &(w, h) in &sides {
            let k: Vec<usize> =
                (0..w * h).map(|_| next() as usize % entries).collect();
            if bits == 8 {
                data.extend(k.iter().map(|&i| i as u8));
            } else {
                data.extend(k.chunks(2).map(|p| {
                    (p[0] << 4 | p.get(1).copied().unwrap_or(0)) as u8
                }));
            }
            indices.push(k);
        }
        g.compressed_tex_image_2d(gl::TEXTURE_2D, -3, format, 8, 4, 0, &data);
        assert_eq!(g.get_error(), gl::NO_ERROR, "{format:04x}");
        let d = g.textures().unwrap().desc(name[0]).unwrap();
        let words = g.textures().unwrap().words();
        for (l, &(w, h)) in sides.iter().enumerate() {
            let l = l as u32;
            for j in 0..h {
                for i in 0..w {
                    let k = indices[l as usize][(j * w + i) as usize];
                    let want = channels(&data[k * entry..k * entry + entry]);
                    let at = razboj_tile::tex::level_base(&d, l)
                        + texel_offset(&d, l, i, j);
                    let got = words[((at - BUS) / 4) as usize];
                    // The word is alpha, red, green, blue from the top.
                    for (c, &(v, n)) in want.iter().enumerate() {
                        let g8 = (got >> [16, 8, 0, 24][c]) & 0xff;
                        let w8 = (v as f64 * 255.0 / ((1 << n) - 1) as f64)
                            .round() as i32;
                        assert!(
                            (g8 as i32 - w8).abs() <= 1,
                            "{format:04x} level {l} ({i},{j}) channel {c}: \
                             {g8} not {w8}"
                        );
                    }
                }
            }
        }
    }
    // The levels' data cut short, and a positive level.
    let short = [0u8; 16 * 3 + 4];
    g.compressed_tex_image_2d(
        gl::TEXTURE_2D,
        -3,
        gl::PALETTE4_RGB8_OES,
        8,
        4,
        0,
        &short,
    );
    assert_eq!(g.get_error(), gl::INVALID_VALUE);
    g.compressed_tex_image_2d(
        gl::TEXTURE_2D,
        1,
        gl::PALETTE4_RGB8_OES,
        8,
        4,
        0,
        &[0u8; 256],
    );
    assert_eq!(g.get_error(), gl::INVALID_VALUE);
}

/// A point sprite (#998, `OES_point_sprite`): with `GL_POINT_SPRITE_OES`
/// on and `GL_COORD_REPLACE_OES`, a point 32 pixels square is the
/// texture across it, each pixel the texel GL's formula puts there,
/// `s = 1/2 + (x - x_w + 1/2) / size` and `t = 1/2 - (y - y_w + 1/2) /
/// size`, at the nearest under `GL_REPLACE`; without coordinate
/// replacement it is the point's own colour, untextured, as before.
#[test]
fn a_point_sprite_is_the_texture_across_it() {
    let draw = |replace: bool| {
        let mut frame = vec![[0u32; WORDS]; 64];
        let room: &'static mut [u32] =
            Box::leak(vec![0u32; 1 << 16].into_boxed_slice());
        let mut g = Gl::new(&mut frame, W, H);
        g.texture_room(room, BUS);
        let mut name = [0u32];
        g.gen_textures(&mut name);
        g.bind_texture(gl::TEXTURE_2D, name[0]);
        let t2 = gl::TEXTURE_2D;
        g.tex_parameter(t2, gl::TEXTURE_MIN_FILTER, gl::NEAREST);
        g.tex_parameter(t2, gl::TEXTURE_MAG_FILTER, gl::NEAREST);
        let img = image();
        let (s, rgba, ub) = (SIDE as u32, gl::RGBA, gl::UNSIGNED_BYTE);
        g.tex_image_2d(t2, 0, rgba, s, s, 0, rgba, ub, &img);
        let mode = gl::REPLACE as Fx;
        g.tex_env(gl::TEXTURE_ENV, gl::TEXTURE_ENV_MODE, &[mode]);
        g.enable(gl::TEXTURE_2D);
        g.enable(gl::POINT_SPRITE_OES);
        let on = replace as Fx;
        g.tex_env(gl::POINT_SPRITE_OES, gl::COORD_REPLACE_OES, &[on]);
        assert!(g.is_enabled(gl::POINT_SPRITE_OES));
        g.point_size(32 * ONE);
        g.color(ONE, ONE / 2, 0, ONE);
        g.draw_vertices(gl::POINTS, 1, |_| Vertex {
            position: [0, 0, 0, ONE],
            colour: None,
            normal: None,
            tex: None,
        });
        assert_eq!(g.get_error(), gl::NO_ERROR);
        let n = g.frame().len();
        let words = g.textures().unwrap().words().to_vec();
        let list = decode_list(&frame[..n]);
        let read = |a: u32| words[((a - BUS) / 4) as usize];
        let t = Textures { mem: &read };
        render_textured(
            &list,
            W as usize,
            H as usize,
            vec![0; 128 * 96],
            Some(&t),
        )
    };
    // The point at the window's centre, (64, 48) with GL's y up, 32
    // square: Razboj's columns 48 to 79 and rows 32 to 63. At that size
    // every pixel's centre is a texel's centre, so no rounding decides.
    let fb = draw(true);
    for j in 0..32usize {
        for i in 0..32usize {
            let (s, t) = ((i as f64 + 0.5) / 32.0, (j as f64 + 0.5) / 32.0);
            let (ti, tj) =
                ((s * SIDE as f64) as usize, (t * SIDE as f64) as usize);
            let k = 4 * (tj * SIDE + ti);
            let b = &image()[k..k + 4];
            let want = (b[3] as u32) << 24
                | (b[0] as u32) << 16
                | (b[1] as u32) << 8
                | b[2] as u32;
            let got = fb[(32 + j) * W as usize + 48 + i];
            assert_eq!(got, want, "pixel {i},{j} of the sprite");
        }
    }
    // Outside the square nothing is drawn.
    assert_eq!(fb.iter().filter(|&&p| p != 0).count(), 1024);
    // Without coordinate replacement, the point's colour, untextured.
    let fb = draw(false);
    let colour = fb[(32 + 7) * W as usize + 48 + 7];
    assert_eq!(colour & 0xff_ffff, 0xff_8000);
    assert_eq!(fb.iter().filter(|&&p| p != 0).count(), 1024);
}
