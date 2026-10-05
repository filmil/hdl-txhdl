// SPDX-License-Identifier: Apache-2.0
//! One frame of `ico_hdmi` as the board shows it, worked out on the
//! host: the list `ico_list` writes, rendered through Razboj's model,
//! with the logo laid in as the program paints it, a pixel of its own
//! to a pixel of the screen (issues 986 and 1213). Written as a PPM to
//! standard output, to hold a recording of the board against.
//!
//!   bazel run //cpu/vreteno/rust:ico_frame -- 40 > frame.ppm
//!
//! The argument is the frame's number; the solid turns as the program
//! turns it, two steps about one axis and one about the other a frame,
//! and an odd frame is drawn into the second frame's rows, as the
//! program draws it, and read back from there.
mod ico_list;

use ico_list::{frame, Box, Solid, BACKDROP, H, MOST, REACH, SECOND, W, WORDS};
use razboj::dl::decode;
use razboj::model::render;
use std::io::Write;

fn main() {
    let n: i32 = std::env::args()
        .nth(1)
        .map(|a| a.parse().expect("a frame number"))
        .unwrap_or(0);
    let (ay, ax) = ((2 * n) & 255, n & 255);
    let mut out = [[0u32; WORDS]; MOST];
    let dy = (n & 1) * SECOND;
    let (k, _) = frame(&Solid::new(), ay, ax, dy, Box::SCREEN, &mut out);
    let ops: Vec<_> = out[..k].iter().map(|w| decode(w)).collect();
    let all = render(&ops, 1024, (SECOND + H) as usize);
    let mut fb = all[dy as usize * 1024..(dy + H) as usize * 1024].to_vec();
    for p in fb.iter_mut() {
        if *p == 0 {
            *p = BACKDROP;
        }
    }
    let (lx, ly) = (
        W as usize - txhdl_logo::W - 8,
        H as usize - txhdl_logo::H - 8,
    );
    // The solid never reaches the logo, so the frame's clears never
    // touch it, as ico_hdmi asserts too.
    assert!(W / 2 + REACH < lx as i32, "the solid reaches the logo");
    for r in 0..txhdl_logo::H {
        for c in 0..txhdl_logo::W {
            if let Some(px) = txhdl_logo::colour(c, r) {
                fb[(ly + r) * 1024 + lx + c] = px;
            }
        }
    }
    let mut ppm = format!("P6\n{W} {H}\n255\n").into_bytes();
    for y in 0..H as usize {
        for x in 0..W as usize {
            let p = fb[y * 1024 + x];
            ppm.extend([(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        }
    }
    std::io::stdout().write_all(&ppm).expect("standard output");
}
