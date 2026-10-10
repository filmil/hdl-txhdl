// SPDX-License-Identifier: Apache-2.0
//! One frame of the textured GL icosahedron (#997) as the board shows
//! it, worked out on the host: `ico_gl::frame` with the depth test and
//! the checker, its texture uploaded into a room as the board's is, the
//! list rendered through Razboj's model with that room as its memory,
//! and the logo laid in. Written as a PNG to the file the second
//! argument names, to look at and to hold a recording of the board
//! against.
//!
//!   bazel run //cpu/vreteno/rust:ico_tex_frame -- 40 $PWD/frame.png
//!
//! The first argument is the frame's number; the solid turns as the
//! program turns it, and an odd frame is drawn into the second frame's
//! rows, as the program draws it, and read back from there.
// Both are shared with the board program, which uses parts of them
// this does not.
#[allow(dead_code)]
mod ico_gl;
#[allow(dead_code)]
mod ico_list;

use ico_gl::{Model, TEX_ROOM};
use ico_list::{Box, Solid, BACKDROP, H, SECOND, W, WORDS};
use razboj::dl::decode_list;
use razboj::model::{render_textured, Textures};

/// The texture's room's bus address: the board's, two megabytes into
/// the list's memory.
const TEX: u32 = 0x42a0_0000;

/// The framebuffer as the board has it: rows of 1024 words.
const FW: usize = 1024;

fn main() {
    let mut args = std::env::args().skip(1);
    let n: i32 = args
        .next()
        .map(|a| a.parse().expect("a frame number"))
        .unwrap_or(0);
    let path = args.next().unwrap_or_else(|| "ico_tex.png".into());
    let (ay, ax) = ((2 * n) & 255, n & 255);
    let dy = (n & 1) * SECOND;
    let model = Model::new(&Solid::new());
    let mut room = vec![0u32; TEX_ROOM];
    let mut out = [[0u32; WORDS]; ico_gl::MOST];
    let (k, _) = ico_gl::frame(
        &model,
        ay,
        ax,
        dy,
        Box::SCREEN,
        true,
        // Smooth shading's frame is #1592's step 3, untextured.
        (!cfg!(smooth)).then_some((&mut room[..], TEX, true)),
        &mut out,
    );
    let list = decode_list(&out[..k]);
    let read = |a: u32| room[((a - TEX) / 4) as usize];
    let t = Textures { mem: &read };
    let rows = (SECOND + H) as usize;
    let all = render_textured(&list, FW, rows, vec![0; FW * rows], Some(&t));
    let mut fb: Vec<u32> = (0..H as usize)
        .flat_map(|y| {
            let at = (dy as usize + y) * FW;
            all[at..at + W as usize].to_vec()
        })
        .collect();
    for p in fb.iter_mut() {
        if *p == 0 {
            *p = BACKDROP;
        }
    }
    let (lx, ly) = (
        W as usize - txhdl_logo::W - 8,
        H as usize - txhdl_logo::H - 8,
    );
    for r in 0..txhdl_logo::H {
        for c in 0..txhdl_logo::W {
            if let Some(px) = txhdl_logo::colour(c, r) {
                fb[(ly + r) * W as usize + lx + c] = px;
            }
        }
    }
    let png = razboj::image::png(&fb, W as usize, H as usize, 1);
    std::fs::write(&path, png).expect("the PNG's file");
    let textured = list.iter().filter(|i| i.tex.to_bool()).count();
    println!("frame {n}: {k} slots, {textured} textured faces, {path}");
}
