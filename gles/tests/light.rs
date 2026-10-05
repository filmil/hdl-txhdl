// SPDX-License-Identifier: Apache-2.0
//! Lighting against floating point (issue 1164): the specification's sum
//! at a vertex written again with `f64`, and the library's colours within
//! one step of the byte each becomes.
use gles::fixed::{Fx, ONE};
use gles::gl;
use gles::light::{shade, Light, Material};
use gles::{colour_word, Gl};
use razboj::dl::decode;
use razboj_tile::WORDS;

fn f(v: Fx) -> f64 {
    v as f64 / 65536.0
}

fn fx(v: f64) -> Fx {
    (v * 65536.0).round() as Fx
}

/// The specification's sum, in floating point, with the viewer at
/// infinity.
fn reference(
    n: [f64; 3],
    v: [f64; 3],
    m: &Material,
    lights: &[Light],
    scene: [Fx; 4],
) -> [f64; 4] {
    let c = |a: &[Fx; 4]| [f(a[0]), f(a[1]), f(a[2]), f(a[3])];
    let dot =
        |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let unit = |a: [f64; 3]| {
        let l = dot(a, a).sqrt();
        if l == 0.0 {
            a
        } else {
            [a[0] / l, a[1] / l, a[2] / l]
        }
    };
    let (ma, md, ms, me) =
        (c(&m.ambient), c(&m.diffuse), c(&m.specular), c(&m.emission));
    let s = c(&scene);
    let mut out = [0.0; 3];
    for i in 0..3 {
        out[i] = me[i] + s[i] * ma[i];
    }
    for l in lights.iter().filter(|l| l.on) {
        let p = c(&l.position);
        let (to, att) = if l.position[3] == 0 {
            (unit([p[0], p[1], p[2]]), 1.0)
        } else {
            let vp =
                [p[0] / p[3] - v[0], p[1] / p[3] - v[1], p[2] / p[3] - v[2]];
            let d = dot(vp, vp).sqrt();
            let [k0, k1, k2] = l.attenuation.map(f);
            (unit(vp), 1.0 / (k0 + k1 * d + k2 * d * d))
        };
        let spot = if l.spot_cutoff == 180 * ONE {
            1.0
        } else {
            let sd = unit(l.spot_direction.map(f));
            let s = dot([-to[0], -to[1], -to[2]], sd);
            if s < f(l.spot_cutoff).to_radians().cos() {
                0.0
            } else {
                s.max(0.0).powf(f(l.spot_exponent))
            }
        };
        let nl = dot(n, to).max(0.0);
        let h = unit([to[0], to[1], to[2] + 1.0]);
        let sh = if nl > 0.0 {
            let x = dot(n, h).clamp(0.0, 1.0);
            if m.shininess == 0 {
                1.0
            } else {
                x.powf(f(m.shininess))
            }
        } else {
            0.0
        };
        let (la, ld, ls) = (c(&l.ambient), c(&l.diffuse), c(&l.specular));
        for i in 0..3 {
            out[i] += att
                * spot
                * (la[i] * ma[i] + nl * ld[i] * md[i] + sh * ls[i] * ms[i]);
        }
    }
    [
        out[0].clamp(0.0, 1.0),
        out[1].clamp(0.0, 1.0),
        out[2].clamp(0.0, 1.0),
        md[3].clamp(0.0, 1.0),
    ]
}

/// Each byte of the library's colour within one of the reference's.
fn near(got: u32, want: [f64; 4], what: &str) {
    for (k, shift) in [24, 16, 8, 0].iter().enumerate() {
        let g = (got >> shift) & 0xff;
        let w = [want[3], want[0], want[1], want[2]][k] * 255.0;
        assert!(
            (g as f64 - w).abs() <= 1.0,
            "{what}: byte {k} is {g}, want {w:.2}"
        );
    }
}

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    fn unit(&mut self) -> f64 {
        (self.next() % 100_001) as f64 / 100_000.0
    }
}

/// The sum at a vertex for random unit normals, materials and lights,
/// directional and positional, with spots, attenuation and shininess.
#[test]
fn the_sum_at_a_vertex_agrees_with_floating_point() {
    let mut r = Rng(0x5eed_1164);
    for case in 0..3000 {
        let col = |r: &mut Rng| {
            [fx(r.unit()), fx(r.unit()), fx(r.unit()), fx(r.unit())]
        };
        let m = Material {
            ambient: col(&mut r),
            diffuse: col(&mut r),
            specular: col(&mut r),
            emission: col(&mut r).map(|c| c / 4),
            shininess: fx(r.unit() * 128.0),
        };
        let mut lights = [Light::new(0), Light::new(1)];
        for (k, l) in lights.iter_mut().enumerate() {
            l.on = k == 0 || r.next() & 1 == 1;
            l.ambient = col(&mut r).map(|c| c / 4);
            l.diffuse = col(&mut r);
            l.specular = col(&mut r);
            let positional = r.next().is_multiple_of(3);
            let p =
                [r.unit() * 4.0 - 2.0, r.unit() * 4.0 - 2.0, r.unit() * 4.0];
            l.position = [
                fx(p[0]),
                fx(p[1]),
                fx(p[2]),
                if positional { ONE } else { 0 },
            ];
            if positional {
                l.attenuation = [
                    fx(0.5 + r.unit()),
                    fx(r.unit() * 0.3),
                    fx(r.unit() * 0.1),
                ];
            }
            if r.next().is_multiple_of(4) {
                l.spot_direction = [fx(-p[0]), fx(-p[1]), fx(-p[2])];
                l.spot_cutoff = fx(20.0 + r.unit() * 60.0);
                l.spot_exponent = fx(r.unit() * 8.0);
            }
        }
        let n = {
            let v = [
                r.unit() * 2.0 - 1.0,
                r.unit() * 2.0 - 1.0,
                r.unit() * 2.0 - 0.5,
            ];
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            [v[0] / l, v[1] / l, v[2] / l]
        };
        let v = [r.unit() - 0.5, r.unit() - 0.5, -r.unit() * 2.0];
        let scene = [fx(0.2), fx(0.2), fx(0.2), ONE];
        let got = shade(
            &n.map(fx),
            &[fx(v[0]), fx(v[1]), fx(v[2]), ONE],
            &m,
            &lights,
            &scene,
        );
        near(
            colour_word(&got),
            reference(n, v, &m, &lights, scene),
            &format!("case {case}"),
        );
    }
}

/// A frame of one flat triangle drawn by `draw`, and the colour it
/// wrote.
fn drawn(draw: impl FnOnce(&mut Gl)) -> u32 {
    let mut frame = vec![[0u32; WORDS]; 16];
    let mut gl = Gl::new(&mut frame, 640, 480);
    gl.shade_model(gl::FLAT);
    gl.enable(gl::LIGHTING);
    gl.enable(gl::LIGHT0);
    draw(&mut gl);
    assert_eq!(gl.get_error(), gl::NO_ERROR);
    let w = gl.frame().last().copied().expect("a triangle drawn");
    let i = decode(&w);
    let colour = ((i.alpha.raw() as u32) << 24) | i.colour.raw() as u32;
    // Drawn by Razboj's model, the triangle's middle is that colour.
    let list: Vec<_> = gl.frame().iter().map(|w| decode(w)).collect();
    let fb = razboj::model::render(&list, 640, 480);
    assert_eq!(fb[260 * 640 + 320], colour, "the pixel the model drew");
    colour
}

/// A triangle facing the eye, in front of it and inside the clip volume
/// of the identity projection, with its normal given.
fn triangle(gl: &mut Gl, n: [f64; 3]) {
    let p = |x: f64, y: f64| [fx(x), fx(y), fx(-0.25), ONE];
    let pos = [p(-0.5, -0.5), p(0.5, -0.5), p(0.0, 0.5)];
    gl.draw_arrays(gl::TRIANGLES, &pos, None, Some(&[n.map(fx); 3]));
}

/// The default light 0 is white and directional along the eye's z, and
/// the default material grey: a normal facing the eye is lit by the
/// diffuse term in full, plus the scene's ambient.
#[test]
fn the_default_light_and_material() {
    let got = drawn(|gl| triangle(gl, [0.0, 0.0, 1.0]));
    let want = reference(
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -2.0],
        &Material::default(),
        &[Light {
            on: true,
            ..Light::new(0)
        }],
        [fx(0.2), fx(0.2), fx(0.2), ONE],
    );
    near(got, want, "facing");
    // 0.2 * 0.2 + 0.8 = 0.84 of 255 is 214.
    assert_eq!(got & 0xff, 214);
}

/// A light's position is carried by the modelview when it is set, not
/// when it is used: set under a turn of ninety degrees about y, a light
/// along z comes from x.
#[test]
fn a_light_is_placed_by_the_modelview_of_its_call() {
    let got = drawn(|gl| {
        gl.push_matrix();
        gl.rotate(fx(90.0), 0, ONE, 0);
        gl.light(gl::LIGHT0, gl::POSITION, &[0, 0, ONE, 0]);
        gl.pop_matrix();
        triangle(gl, [1.0, 0.0, 0.0]);
    });
    let mut l = Light {
        on: true,
        ..Light::new(0)
    };
    l.position = [ONE, 0, 0, 0];
    let want = reference(
        [1.0, 0.0, 0.0],
        [0.0, 0.0, -2.0],
        &Material::default(),
        &[l],
        [fx(0.2), fx(0.2), fx(0.2), ONE],
    );
    near(got, want, "from x");
    assert!(got & 0xff > 200, "lit from the side: {got:#x}");
}

/// Under a modelview that scales by two, a unit normal comes out half as
/// long, and the diffuse term with it, unless GL_NORMALIZE or
/// GL_RESCALE_NORMAL makes it unit again.
#[test]
fn scaled_normals_and_their_two_cures() {
    let scene = [fx(0.2), fx(0.2), fx(0.2), ONE];
    let l = [Light {
        on: true,
        ..Light::new(0)
    }];
    let lit = |len: f64| {
        reference(
            [0.0, 0.0, len],
            [0.0, 0.0, -4.0],
            &Material::default(),
            &l,
            scene,
        )
    };
    let at = |cure: Option<u32>| {
        drawn(|gl| {
            gl.scale(2 * ONE, 2 * ONE, 2 * ONE);
            if let Some(c) = cure {
                gl.enable(c);
            }
            triangle(gl, [0.0, 0.0, 1.0]);
        })
    };
    near(at(None), lit(0.5), "scaled");
    near(at(Some(gl::NORMALIZE)), lit(1.0), "normalized");
    near(at(Some(gl::RESCALE_NORMAL)), lit(1.0), "rescaled");
}

/// With GL_COLOR_MATERIAL the vertex's colour is the material's ambient
/// and diffuse; with two-sided lighting a face turned away is lit for
/// its back, so a normal facing away still shows.
#[test]
fn colour_material_and_two_sided_lighting() {
    let scene = [fx(0.2), fx(0.2), fx(0.2), ONE];
    let l = [Light {
        on: true,
        ..Light::new(0)
    }];
    let red = [fx(0.9), fx(0.1), fx(0.3), fx(0.5)];
    let got = drawn(|gl| {
        gl.enable(gl::COLOR_MATERIAL);
        gl.color(red[0], red[1], red[2], red[3]);
        triangle(gl, [0.0, 0.0, 1.0]);
    });
    let m = Material {
        ambient: red,
        diffuse: red,
        ..Material::default()
    };
    near(
        got,
        reference([0.0, 0.0, 1.0], [0.0, 0.0, -2.0], &m, &l, scene),
        "colour material",
    );
    // Wound clockwise, so its back faces the eye; its normal points away.
    let back = |two: bool| {
        drawn(|gl| {
            gl.light_model(gl::LIGHT_MODEL_TWO_SIDE, &[two as Fx]);
            let p = |x: f64, y: f64| [fx(x), fx(y), fx(-0.25), ONE];
            let pos = [p(-0.5, -0.5), p(0.0, 0.5), p(0.5, -0.5)];
            gl.draw_arrays(gl::TRIANGLES, &pos, None, Some(&[[0, 0, -ONE]; 3]));
        })
    };
    let dark = reference(
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -2.0],
        &Material::default(),
        &l,
        scene,
    );
    let lit = reference(
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -2.0],
        &Material::default(),
        &l,
        scene,
    );
    near(back(false), dark, "one-sided, its back dark");
    near(back(true), lit, "two-sided, its back lit");
}

/// The specification's ranges are refused with GL_INVALID_VALUE, and a
/// light past the eighth with GL_INVALID_ENUM.
#[test]
fn values_out_of_range_are_refused() {
    let mut frame = vec![[0u32; WORDS]; 4];
    let mut gl = Gl::new(&mut frame, 640, 480);
    gl.light(gl::LIGHT0, gl::SPOT_CUTOFF, &[fx(120.0)]);
    assert_eq!(gl.get_error(), gl::INVALID_VALUE);
    gl.light(gl::LIGHT0, gl::SPOT_EXPONENT, &[fx(129.0)]);
    assert_eq!(gl.get_error(), gl::INVALID_VALUE);
    gl.material(gl::FRONT_AND_BACK, gl::SHININESS, &[fx(-1.0)]);
    assert_eq!(gl.get_error(), gl::INVALID_VALUE);
    gl.light(gl::LIGHT0 + 8, gl::AMBIENT, &[0; 4]);
    assert_eq!(gl.get_error(), gl::INVALID_ENUM);
    gl.material(gl::FRONT, gl::SHININESS, &[ONE]);
    assert_eq!(gl.get_error(), gl::INVALID_ENUM);
    gl.light(gl::LIGHT0, gl::SPOT_CUTOFF, &[fx(180.0)]);
    assert_eq!(gl.get_error(), gl::NO_ERROR);
}
