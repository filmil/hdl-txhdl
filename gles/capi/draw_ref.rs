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
    g.clear_depth(ONE / 2);
    g.clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
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
    g.enable(gl::DEPTH_TEST);
    g.depth_func(gl::LEQUAL);
    g.depth_mask(false);
    g.depth_range(ONE / 4, 3 * ONE / 4);
    g.enable(gl::BLEND);
    g.blend_func(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
    g.enable(gl::ALPHA_TEST);
    g.alpha_func(gl::GREATER, ONE / 4);
    g.color_mask(true, true, true, false);
    g.draw_arrays(gl::TRIANGLE_FAN, &fan, None, None);

    let words = g.frame();
    println!("frame {}", words.len());
    for w in words.iter().flatten() {
        println!("{w:08x}");
    }
    println!("error {:04x}", g.get_error());
    println!("unimplemented {:04x}", gl::INVALID_OPERATION);
    println!(
        "version OpenGL ES-CL 1.1 TxHDL, Common-Lite, one texture unit, not conformant"
    );
    // The queries (#1484), of the state set above.
    let mut v = [0i32; 4];
    g.get_integer(gles::get::name::VIEWPORT, &mut v);
    println!("viewport {} {} {} {}", v[0], v[1], v[2], v[3]);
    g.get_integer(gles::get::name::DEPTH_FUNC, &mut v);
    println!("depth func {:04x}", v[0]);
    let mut mask = [false; 4];
    g.get_boolean(gles::get::name::COLOR_WRITEMASK, &mut mask);
    let m = mask.map(|b| b as u8);
    println!("colour mask {}{}{}{}", m[0], m[1], m[2], m[3]);
    let mut range = [0; 2];
    g.get_fixed(gles::get::name::DEPTH_RANGE, &mut range);
    println!("depth range {} {}", range[0], range[1]);
    // The fan's array: two fixed components a vertex, packed.
    println!("vertex array 2 140c 0");
    println!("vertex pointer 1");

    // Buffer objects (#1488): the quad in a new context, scaled by a half,
    // its colours per vertex, as draw.c draws it from its buffers.
    let mut frame = [[0u32; 16]; CAP];
    let mut g = Gl::new(&mut frame, 64, 48);
    g.scale(ONE / 2, ONE / 2, ONE);
    g.draw_elements(gl::TRIANGLES, &indices, &positions, Some(&colours), None);
    // Two names, the first store a quad of shorts and its colours.
    println!("buffers 1 2 size 48 is 1");
    // The element buffer, and the vertex array's, the first.
    println!("bindings 2 1");
    let words = g.frame();
    println!("buffer frame {}", words.len());
    for w in words.iter().flatten() {
        println!("{w:08x}");
    }
    println!("error {:04x}", g.get_error());
}
