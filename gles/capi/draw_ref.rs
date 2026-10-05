// SPDX-License-Identifier: Apache-2.0
//! `draw.c`'s scene through the Rust API (issue 1224), printed as
//! `draw.c` prints it, so that `capi_test` can hold the C entry points
//! to it word for word. The arrays are `draw.c`'s, converted to 16.16
//! as GL ES 1.1 says a client array of each type is: a short of a
//! position is a whole number, a byte of a normal is `(2c + 1) / 255`,
//! and a byte of a colour is `c / 255`.
use gles::fixed::{Fx, ONE};
use gles::{gl, Gl};

const CAP: usize = 64;

fn unit(c: i64) -> Fx {
    ((c * ONE as i64 + 127) / 255) as Fx
}

fn signed_unit(c: i64) -> Fx {
    (((2 * c + 1) * ONE as i64) / 255) as Fx
}

fn main() {
    let quad = [[-1i64, -1], [1, -1], [1, 1], [-1, 1]];
    let normals = [
        [-40i64, -40, 100],
        [40, -40, 100],
        [40, 40, 100],
        [-40, 40, 100],
    ];
    let colours = [
        [255i64, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 0, 255],
    ];
    let positions: Vec<[Fx; 4]> = quad
        .iter()
        .map(|p| [(p[0] << 16) as Fx, (p[1] << 16) as Fx, 0, ONE])
        .collect();
    let normals: Vec<[Fx; 3]> =
        normals.iter().map(|n| n.map(signed_unit)).collect();
    let colours: Vec<[Fx; 4]> = colours.iter().map(|c| c.map(unit)).collect();
    let indices = [0u16, 1, 2, 0, 2, 3];
    let fan: Vec<[Fx; 4]> =
        [[0, 0], [ONE / 2, 0], [ONE / 2, ONE / 2], [0, ONE / 2]]
            .iter()
            .map(|p| [p[0], p[1], 0, ONE])
            .collect();

    let mut frame = [[0u32; 16]; CAP];
    let mut g = Gl::new(&mut frame, 64, 48);
    g.clear_color(0, 0, ONE / 4, ONE);
    g.clear(gl::COLOR_BUFFER_BIT);
    g.matrix_mode(gl::PROJECTION);
    g.load_identity();
    g.frustum(-ONE, ONE, -3 * ONE / 4, 3 * ONE / 4, ONE, 10 * ONE);
    g.matrix_mode(gl::MODELVIEW);
    g.load_identity();
    g.light(gl::LIGHT0, gl::POSITION, &[0, 0, ONE, 0]);
    g.enable(gl::LIGHTING);
    g.enable(gl::LIGHT0);
    g.enable(gl::COLOR_MATERIAL);
    g.enable(gl::NORMALIZE);
    g.translate(0, 0, -3 * ONE);
    g.rotate(30 * ONE, 0, ONE, 0);
    g.draw_elements(
        gl::TRIANGLES,
        &indices,
        &positions,
        Some(&colours),
        Some(&normals),
    );
    g.disable(gl::LIGHTING);
    g.shade_model(gl::FLAT);
    g.color(ONE, ONE / 2, 0, ONE);
    g.draw_arrays(gl::TRIANGLE_FAN, &fan, None, None);

    let words = g.frame();
    println!("frame {}", words.len());
    for w in words.iter().flatten() {
        println!("{w:08x}");
    }
    println!("error {:04x}", g.get_error());
    println!("unimplemented {:04x}", gl::INVALID_OPERATION);
    println!(
        "version OpenGL ES-CL 1.1 TxHDL, Common-Lite without textures, not conformant"
    );
}
