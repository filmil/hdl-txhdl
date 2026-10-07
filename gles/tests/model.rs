// SPDX-License-Identifier: Apache-2.0
//! The GL pipeline against references, through Razboj's model (issue
//! 1159).
//!
//! The library's words are checked against Razboj's own encoder, bit for
//! bit. Its pictures are checked against a second pipeline written here
//! with 128-bit integers throughout, to the same rules of rounding, that
//! hands its triangles to Razboj's assembler as `Op`s: the two are drawn
//! by the model and compared pixel for pixel, the library's binned into
//! tiles. And its window positions are checked against floating point,
//! to within a sixteenth of a pixel.
use gles::fixed::{Fx, ONE};
use gles::gl;
use gles::matrix::Mat;
use gles::{colour_word, Gl};
use razboj::dl::{decode, encode, encode_ext};
use razboj::model::render;
use razboj::op::{assemble, DepthMode, Insn, Op};
use razboj_tile::{TILE_WORDS, WORDS};

const W: u32 = 640;
const H: u32 = 480;

fn fx(v: f64) -> Fx {
    (v * 65536.0).round() as Fx
}

fn p(x: f64, y: f64, z: f64) -> [Fx; 4] {
    [fx(x), fx(y), fx(z), ONE]
}

fn rgba(r: f64, g: f64, b: f64, a: f64) -> [Fx; 4] {
    [fx(r), fx(g), fx(b), fx(a)]
}

// ---------------------------------------------------------------------
// The words, against Razboj's encoder.

/// Pseudorandom numbers from a literal seed.
struct Rng(u32);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.next() % (hi - lo + 1) as u32) as i32
    }
}

/// Flat and shaded triangles anywhere in Razboj's range, many hanging off
/// the screen: the library's words are the encoder's, bit for bit, the
/// planes in 64 bits the same numbers as in 128.
#[test]
fn the_words_are_razbojs_encoders() {
    let mut r = Rng(0x1234_5678);
    let screen = (0, 0, W - 1, H - 1);
    let (mut drawn, mut shaded, mut deep) = (0, 0, 0);
    for _ in 0..4000 {
        let v = |r: &mut Rng| (r.range(-16384, 16383), r.range(-16384, 16383));
        let near = |r: &mut Rng| (r.range(-800, 11000), r.range(-800, 8500));
        let pick = r.next() % 3;
        let (a, b, c) = if pick == 0 {
            (v(&mut r), v(&mut r), v(&mut r))
        } else {
            (near(&mut r), near(&mut r), near(&mut r))
        };
        let colours = [r.next(), r.next(), r.next()];
        let smooth = r.next() & 1 == 1;
        // A third of them test depth, under a comparison and a mask of
        // their own, with a depth at each vertex across the range.
        let z = [r.next() & 0xffff, r.next() & 0xffff, r.next() & 0xffff];
        let mode = r.next().is_multiple_of(3).then(|| DepthMode {
            func: r.next() & 7,
            write: r.next() & 1 == 1,
        });
        let ours = gles::emit::triangle(
            colours[0],
            a,
            b,
            c,
            smooth.then_some(colours),
            mode.map(|_| z),
            screen,
        )
        .map(|(mut w, slot)| {
            if let Some(m) = mode {
                gles::emit::depth(&mut w, m.func, m.write);
            }
            (w, slot)
        });
        let op = match (smooth, mode.is_some()) {
            (true, false) => Op::Gouraud { a, b, c, colours },
            (false, false) => Op::TriQ4 {
                colour: colours[0],
                a,
                b,
                c,
            },
            (true, true) => Op::GouraudZ {
                a,
                b,
                c,
                colours,
                z,
            },
            (false, true) => Op::TriZ {
                colour: colours[0],
                a,
                b,
                c,
                z,
            },
        };
        let (sw, sh) = (W as usize, H as usize);
        let theirs = op
            .encode_with(screen, mode, sw, sh)
            .map(|i| (encode(&i), encode_ext(&i)));
        assert_eq!(ours, theirs, "{op:?}");
        drawn += ours.is_some() as u32;
        shaded += (ours.is_some() && smooth) as u32;
        deep += (ours.is_some() && mode.is_some()) as u32;
    }
    assert!(
        drawn > 1500 && shaded > 700 && deep > 400,
        "{drawn} drawn, {shaded} shaded, {deep} with depth"
    );
}

/// Textured triangles (#997): the library's two texture slots are the
/// assembler's, bit for bit, the planes and the level of detail's
/// numerators and their shift, over triangles anywhere in Razboj's range
/// and texture coordinates across a texture of 1024 repeated.
#[test]
fn the_texture_slots_are_razbojs_encoders() {
    use razboj::dl::encode_tex;
    use razboj::op::TexMode;
    let mut r = Rng(0x7e57_0997);
    let screen = (0, 0, W - 1, H - 1);
    let mut drawn = 0;
    for _ in 0..3000 {
        let near = |r: &mut Rng| (r.range(-800, 11000), r.range(-800, 8500));
        let (a, b, c) = (near(&mut r), near(&mut r), near(&mut r));
        let uvq = [0, 1, 2].map(|_| {
            let q = (r.next() as u64 % (1 << 16) + 1) << 32;
            let u = r.range(-4096, 4096) as i64 * (q >> 16) as i64;
            let v = r.range(-4096, 4096) as i64 * (q >> 16) as i64;
            (u, v, q)
        });
        let ours = gles::emit::textured([a, b, c], uvq, screen);
        let ops = [
            Op::Texture(Some(TexMode {
                desc: 0,
                env: 0,
                env_colour: 0,
            })),
            Op::TexTri {
                a,
                b,
                c,
                colours: [0; 3],
                shaded: false,
                z: [0; 3],
                uvq,
            },
        ];
        let theirs = assemble(&ops, W as usize, H as usize)
            .first()
            .and_then(encode_tex);
        assert_eq!(ours, theirs, "{a:?} {b:?} {c:?} {uvq:?}");
        drawn += ours.is_some() as u32;
    }
    assert!(drawn > 1500, "{drawn} drawn");
}

// ---------------------------------------------------------------------
// The reference pipeline, in 128 bits.

/// The state a scene sets, which the reference reads from the context
/// after the scene has set it.
struct State {
    mv: Mat,
    pj: Mat,
    viewport: (i32, i32, i32, i32),
    plane: Option<[Fx; 4]>,
    cull: Option<(u32, u32)>,
    smooth: bool,
}

#[derive(Clone, Copy)]
struct V {
    eye: [i128; 4],
    clip: [i128; 4],
    col: [i128; 4],
}

fn narrow(acc: i128) -> i128 {
    ((acc + (1 << 15)) >> 16).clamp(i32::MIN as i128, i32::MAX as i128)
}

fn times(m: &Mat, v: &[i128; 4]) -> [i128; 4] {
    std::array::from_fn(|row| {
        narrow((0..4).map(|k| m[k * 4 + row] as i128 * v[k]).sum())
    })
}

fn rdiv(n: i128, d: i128) -> i128 {
    let (n, d) = if d < 0 { (-n, -d) } else { (n, d) };
    (2 * n + d).div_euclid(2 * d)
}

const GMIN: i128 = -16384 + 16;
const GMAX: i128 = 16383 - 16;

fn dist(s: &State, v: &V, plane: usize) -> i128 {
    let [x, y, z, w] = v.clip;
    let (vx, vy, vw, vh) = s.viewport;
    let (vx16, vy16, vw8, vh8) = (
        16 * vx as i128,
        16 * vy as i128,
        8 * vw as i128,
        8 * vh as i128,
    );
    let sh16 = 16 * H as i128;
    match plane {
        0 => z + w,
        1 => w - z,
        2 => (0..4).map(|i| s.plane.unwrap()[i] as i128 * v.eye[i]).sum(),
        3 => x * vw8 + w * (vx16 + vw8 - GMIN),
        4 => w * (GMAX - vx16 - vw8) - x * vw8,
        5 => y * vh8 + w * (vy16 + vh8 - (sh16 - GMAX)),
        _ => w * ((sh16 - GMIN) - vy16 - vh8) - y * vh8,
    }
}

fn cross(a: &V, b: &V, da: i128, db: i128) -> V {
    let mix = |p: &[i128; 4], q: &[i128; 4]| {
        std::array::from_fn(|i| {
            (p[i] + rdiv((q[i] - p[i]) * da, da - db))
                .clamp(i32::MIN as i128, i32::MAX as i128)
        })
    };
    V {
        eye: mix(&a.eye, &b.eye),
        clip: mix(&a.clip, &b.clip),
        col: mix(&a.col, &b.col),
    }
}

fn word(c: &[i128; 4]) -> u32 {
    let byte = |v: i128| ((v.clamp(0, 65536) * 255 + (1 << 15)) >> 16) as u32;
    (byte(c[3]) << 24) | (byte(c[0]) << 16) | (byte(c[1]) << 8) | byte(c[2])
}

/// One triangle through the reference, as Razboj's `Op`s.
fn reference(s: &State, tri: [V; 3], out: &mut Vec<Op>) {
    let provoking = tri[2].col;
    let mut poly: Vec<V> = tri.to_vec();
    let guard = (3..7).any(|p| tri.iter().any(|v| dist(s, v, p) < 0));
    for p in 0..7 {
        if (p == 2 && s.plane.is_none()) || (p >= 3 && !guard) {
            continue;
        }
        let mut next = vec![];
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let (da, db) = (dist(s, &a, p), dist(s, &b, p));
            if da >= 0 {
                next.push(a);
            }
            if (da >= 0) != (db >= 0) {
                next.push(cross(&a, &b, da, db));
            }
        }
        poly = next;
        if poly.len() < 3 {
            return;
        }
    }
    let (vx, vy, vw, vh) = s.viewport;
    let mut win = vec![];
    for v in &poly {
        let [x, y, _, w] = v.clip;
        if w <= 0 {
            return;
        }
        win.push((
            16 * vx as i128 + 8 * vw as i128 + rdiv(x * 8 * vw as i128, w),
            16 * vy as i128 + 8 * vh as i128 + rdiv(y * 8 * vh as i128, w),
        ));
    }
    let n = win.len();
    let area: i128 = (0..n)
        .map(|k| win[k].0 * win[(k + 1) % n].1 - win[(k + 1) % n].0 * win[k].1)
        .sum();
    if area == 0 {
        return;
    }
    if let Some((cull, front_face)) = s.cull {
        let front = (area > 0) == (front_face == gl::CCW);
        if cull == gl::FRONT_AND_BACK
            || (cull == gl::FRONT && front)
            || (cull == gl::BACK && !front)
        {
            return;
        }
    }
    let at = |k: usize| {
        let r = |v: i128| v.clamp(-16384, 16383) as i32;
        (r(win[k].0), r(16 * H as i128 - win[k].1))
    };
    for k in 1..n - 1 {
        let (a, b, c) = (at(0), at(k), at(k + 1));
        out.push(if s.smooth {
            Op::Gouraud {
                a,
                b,
                c,
                colours: [
                    word(&poly[0].col),
                    word(&poly[k].col),
                    word(&poly[k + 1].col),
                ],
            }
        } else {
            Op::TriQ4 {
                colour: word(&provoking),
                a,
                b,
                c,
            }
        });
    }
}

/// A draw call: a mode, its vertices, their colours, and indices or
/// none.
struct Draw {
    mode: u32,
    pos: Vec<[Fx; 4]>,
    col: Vec<[Fx; 4]>,
    idx: Option<Vec<u16>>,
}

/// The triangles of a draw, as the specification orders them.
fn triangles(d: &Draw) -> Vec<[usize; 3]> {
    let at = |k: usize| d.idx.as_ref().map_or(k, |i| i[k] as usize);
    let n = d.idx.as_ref().map_or(d.pos.len(), |i| i.len());
    let mut t = vec![];
    match d.mode {
        gl::TRIANGLES => (0..n / 3)
            .for_each(|i| t.push([at(3 * i), at(3 * i + 1), at(3 * i + 2)])),
        gl::TRIANGLE_STRIP => (0..n.saturating_sub(2)).for_each(|i| {
            t.push(if i % 2 == 1 {
                [at(i + 1), at(i), at(i + 2)]
            } else {
                [at(i), at(i + 1), at(i + 2)]
            })
        }),
        _ => (0..n.saturating_sub(2))
            .for_each(|i| t.push([at(0), at(i + 1), at(i + 2)])),
    }
    t
}

/// The viewport, the culling as `(cull face, front face)`, and smooth
/// shading or flat: the state a scene sets that the reference does not
/// read back from the context.
type Look = ((i32, i32, i32, i32), Option<(u32, u32)>, bool);

/// A scene drawn both ways: `look` and then `setup` set the context's
/// state, then a clear and the draws. The library's frame, untiled and
/// binned, and the reference's list are drawn by the model, and all
/// three pictures agree. Returns how many instructions the library
/// wrote.
fn agree(
    what: &str,
    look: Look,
    setup: impl Fn(&mut Gl),
    draws: &[Draw],
) -> usize {
    let mut frame = vec![[0u32; WORDS]; 4096];
    let mut gl = Gl::new(&mut frame, W, H);
    let (viewport, cull, smooth) = look;
    state(&mut gl, viewport, cull, smooth);
    setup(&mut gl);
    gl.clear_color(fx(0.1), fx(0.1), fx(0.2), ONE);
    gl.clear(gl::COLOR_BUFFER_BIT);
    for d in draws {
        match &d.idx {
            Some(i) => gl.draw_elements(d.mode, i, &d.pos, Some(&d.col), None),
            None => gl.draw_arrays(d.mode, &d.pos, Some(&d.col), None),
        }
    }
    assert_eq!(gl.get_error(), gl::NO_ERROR, "{what}");
    // The reference reads the state the scene left.
    let s = State {
        mv: gl.modelview(),
        pj: gl.projection(),
        viewport,
        plane: gl.is_enabled(gl::CLIP_PLANE0).then(|| gl.eye_plane()),
        cull,
        smooth,
    };
    let mut ops = vec![Op::Clear {
        colour: colour_word(&[fx(0.1), fx(0.1), fx(0.2), ONE]),
    }];
    for d in draws {
        for t in triangles(d) {
            let v = |i: usize| {
                let pos = d.pos[i].map(|c| c as i128);
                let eye = times(&s.mv, &pos);
                let clip = times(&s.pj, &eye);
                V {
                    eye,
                    clip,
                    col: d.col[i].map(|c| c as i128),
                }
            };
            reference(&s, [v(t[0]), v(t[1]), v(t[2])], &mut ops);
        }
    }
    let want = render(
        &assemble(&ops, W as usize, H as usize),
        W as usize,
        H as usize,
    );
    let untiled: Vec<Insn> = gl.frame().iter().map(|w| decode(w)).collect();
    let n = untiled.len();
    assert_eq!(
        render(&untiled, W as usize, H as usize),
        want,
        "{what}: the frame"
    );
    let mut entries = vec![[0u32; WORDS]; 65535];
    let mut tiles = vec![[0u32; TILE_WORDS]; 256];
    let b = gl.flush(&mut entries, &mut tiles).expect("room");
    let binned: Vec<Insn> =
        entries[..b.entries].iter().map(|w| decode(w)).collect();
    assert_eq!(
        render(&binned, W as usize, H as usize),
        want,
        "{what}: binned"
    );
    assert!(gl.frame().is_empty(), "{what}: a new frame begun");
    n
}

/// Sets the look on the context.
fn state(
    gl: &mut Gl,
    viewport: (i32, i32, i32, i32),
    cull: Option<(u32, u32)>,
    smooth: bool,
) {
    gl.viewport(viewport.0, viewport.1, viewport.2, viewport.3);
    if let Some((c, f)) = cull {
        gl.enable(gl::CULL_FACE);
        gl.cull_face(c);
        gl.front_face(f);
    }
    gl.shade_model(if smooth { gl::SMOOTH } else { gl::FLAT });
}

/// A cube of side 2 as twelve triangles, wound counter-clockwise seen
/// from outside, each corner its own colour.
fn cube() -> Draw {
    let c = |i: usize| {
        p(
            if i & 1 == 0 { -1.0 } else { 1.0 },
            if i & 2 == 0 { -1.0 } else { 1.0 },
            if i & 4 == 0 { -1.0 } else { 1.0 },
        )
    };
    let pos: Vec<[Fx; 4]> = (0..8).map(c).collect();
    let col = (0..8)
        .map(|i| {
            rgba(
                (i & 1) as f64,
                ((i >> 1) & 1) as f64,
                ((i >> 2) & 1) as f64 * 0.8 + 0.1,
                1.0,
            )
        })
        .collect();
    let faces: [[u16; 4]; 6] = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    let idx = faces
        .iter()
        .flat_map(|f| [f[0], f[1], f[2], f[0], f[2], f[3]])
        .collect();
    Draw {
        mode: gl::TRIANGLES,
        pos,
        col,
        idx: Some(idx),
    }
}

/// A perspective view of a turned cube, a second cube through the near
/// plane, and a strip behind the eye; smooth and flat, culled and not.
#[test]
fn perspective_with_the_near_plane() {
    for (smooth, cull) in [
        (true, None),
        (false, Some((gl::BACK, gl::CCW))),
        (true, Some((gl::FRONT, gl::CW))),
    ] {
        let setup = |gl: &mut Gl| {
            gl.matrix_mode(gl::PROJECTION);
            gl.frustum(
                fx(-1.0),
                fx(1.0),
                fx(-0.75),
                fx(0.75),
                fx(1.0),
                fx(40.0),
            );
            gl.matrix_mode(gl::MODELVIEW);
            gl.translate(fx(0.3), fx(-0.2), fx(-4.0));
            gl.rotate(fx(33.0), fx(1.0), fx(1.0), fx(0.2));
        };
        let mut near = cube();
        // Its vertices moved towards the eye, so that some lie behind the
        // near plane and some in front.
        for v in &mut near.pos {
            v[2] += fx(3.4);
            v[0] += fx(0.7);
        }
        let behind = Draw {
            mode: gl::TRIANGLE_STRIP,
            pos: vec![
                p(-1.0, -1.0, 6.0),
                p(1.0, -1.0, 6.0),
                p(-1.0, 1.0, 6.0),
                p(1.0, 1.0, 6.0),
            ],
            col: vec![rgba(1.0, 0.0, 0.0, 1.0); 4],
            idx: None,
        };
        let n = agree(
            &format!("perspective {smooth} {cull:?}"),
            ((0, 0, W as i32, H as i32), cull, smooth),
            setup,
            &[cube(), near, behind],
        );
        assert!(n > 6, "the cubes drew: {n}");
    }
}

/// Orthographic triangles thousands of pixels off the screen, which only
/// the guard band keeps inside Razboj's range, and one wholly off it.
#[test]
fn the_guard_band() {
    let setup = |gl: &mut Gl| {
        gl.matrix_mode(gl::PROJECTION);
        gl.ortho(0, fx(640.0), 0, fx(480.0), fx(-1.0), fx(1.0));
        gl.matrix_mode(gl::MODELVIEW);
    };
    let big = Draw {
        mode: gl::TRIANGLES,
        pos: vec![
            p(-3000.0, -200.0, 0.0),
            p(2500.0, 100.0, 0.0),
            p(300.0, 4000.0, 0.0),
            p(100.0, 50.0, 0.0),
            p(5000.0, 400.0, 0.0),
            p(200.0, -2600.0, 0.0),
            p(-5000.0, -5000.0, 0.0),
            p(-4000.0, -5000.0, 0.0),
            p(-4500.0, -4000.0, 0.0),
        ],
        col: vec![
            rgba(1.0, 0.0, 0.0, 1.0),
            rgba(0.0, 1.0, 0.0, 1.0),
            rgba(0.0, 0.0, 1.0, 1.0),
            rgba(1.0, 1.0, 0.0, 0.5),
            rgba(0.0, 1.0, 1.0, 0.5),
            rgba(1.0, 0.0, 1.0, 0.5),
            rgba(1.0, 1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0, 1.0),
        ],
        idx: None,
    };
    let n = agree(
        "the guard band",
        ((0, 0, W as i32, H as i32), None, true),
        setup,
        &[big],
    );
    assert!(n > 3, "the two big triangles were cut into pieces: {n}");
}

/// The user plane through a strip and a fan, in an offset viewport,
/// flat shaded.
#[test]
fn a_user_plane_in_a_viewport() {
    let setup = |gl: &mut Gl| {
        gl.matrix_mode(gl::PROJECTION);
        gl.frustum(fx(-0.5), fx(0.5), fx(-0.375), fx(0.375), fx(1.0), fx(20.0));
        gl.matrix_mode(gl::MODELVIEW);
        gl.translate(0, 0, fx(-3.0));
        gl.rotate(fx(-20.0), 0, ONE, 0);
        gl.clip_plane(gl::CLIP_PLANE0, &[fx(1.0), fx(0.5), 0, fx(-0.2)]);
        gl.enable(gl::CLIP_PLANE0);
    };
    let strip = Draw {
        mode: gl::TRIANGLE_STRIP,
        pos: (0..12)
            .map(|i| {
                p(
                    -1.5 + 0.3 * i as f64,
                    if i % 2 == 0 { -1.0 } else { 0.2 },
                    0.1 * i as f64,
                )
            })
            .collect(),
        col: (0..12)
            .map(|i| rgba(0.1 * i as f64, 0.5, 1.0 - 0.08 * i as f64, 1.0))
            .collect(),
        idx: None,
    };
    let fan = Draw {
        mode: gl::TRIANGLE_FAN,
        pos: (0..9)
            .map(|i| {
                if i == 0 {
                    p(0.0, 0.8, 0.0)
                } else {
                    let a = (i as f64 - 1.0) * 0.6;
                    p(a.cos() * 1.2, 0.8 + a.sin() * 0.7, 0.0)
                }
            })
            .collect(),
        col: (0..9)
            .map(|i| rgba(1.0, 0.1 * i as f64, 0.2, 1.0))
            .collect(),
        idx: Some(vec![0, 1, 2, 3, 4, 5, 6, 7, 8]),
    };
    agree(
        "the user plane",
        ((100, 50, 320, 240), Some((gl::BACK, gl::CCW)), false),
        setup,
        &[strip, fan],
    );
}

// ---------------------------------------------------------------------
// The window, against floating point.

/// Triangles in front of the eye and inside the screen land within a
/// sixteenth of a pixel of where floating point puts them, y turned
/// over for Razboj.
#[test]
fn window_positions_agree_with_floating_point() {
    let mut frame = vec![[0u32; WORDS]; 64];
    let mut gl = Gl::new(&mut frame, W, H);
    gl.matrix_mode(gl::PROJECTION);
    gl.frustum(fx(-1.0), fx(1.0), fx(-0.75), fx(0.75), fx(1.0), fx(40.0));
    gl.matrix_mode(gl::MODELVIEW);
    let mut r = Rng(0xdead_beef);
    let mut checked = 0;
    for _ in 0..200 {
        let f = |r: &mut Rng, lo: f64, hi: f64| {
            lo + (r.next() % 10000) as f64 / 10000.0 * (hi - lo)
        };
        let tri: Vec<(f64, f64, f64)> = (0..3)
            .map(|_| {
                (
                    f(&mut r, -2.0, 2.0),
                    f(&mut r, -1.5, 1.5),
                    f(&mut r, -8.0, -3.0),
                )
            })
            .collect();
        let pos: Vec<[Fx; 4]> = tri.iter().map(|v| p(v.0, v.1, v.2)).collect();
        gl.draw_arrays(gl::TRIANGLES, &pos, None, None);
        let Some(w) = gl.frame().last().copied() else {
            continue;
        };
        let mut entries = vec![[0u32; WORDS]; 512];
        let mut tiles = vec![[0u32; TILE_WORDS]; 256];
        gl.flush(&mut entries, &mut tiles).unwrap();
        let insn = decode(&w);
        let got: Vec<(f64, f64)> =
            [(insn.ax, insn.ay), (insn.bx, insn.by), (insn.cx, insn.cy)]
                .iter()
                .map(|(x, y)| {
                    (
                        razboj::op::signed(*x) as f64,
                        razboj::op::signed(*y) as f64,
                    )
                })
                .collect();
        for v in &tri {
            // x' = x / -z on the near plane at 1, so the window is
            // (x' + 1) * 320 pixels, and y' / 0.75 likewise, turned over.
            let wx = (v.0 / -v.2 + 1.0) * 320.0 * 16.0;
            let wy = 480.0 * 16.0 - (v.1 / -v.2 / 0.75 + 1.0) * 240.0 * 16.0;
            let best = got
                .iter()
                .map(|g| (g.0 - wx).abs().max((g.1 - wy).abs()))
                .fold(f64::MAX, f64::min);
            assert!(best <= 1.0, "{v:?} at ({wx}, {wy}), the library {got:?}");
        }
        checked += 1;
    }
    assert!(checked > 150, "{checked} triangles checked");
}
