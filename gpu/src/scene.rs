// SPDX-License-Identifier: Apache-2.0
//! The scenes the demonstration and the traced run draw.
use crate::op::Op;

// begin{scene}
/// The demonstration: a house under a hill, with a sun of two
/// triangles, on a screen of `w` by `h`.
pub fn house(w: usize, h: usize) -> Vec<Op> {
    let sky = 0x10_1830;
    let hill = 0x37_474f;
    let grass = 0x2e_7d32;
    let wall = 0x8d_6e63;
    let roof = 0xc6_2828;
    let door = 0x4e_342e;
    let sun = 0xfd_d835;
    vec![
        Op::clear(w, h, sky),
        Op::tri((38, 48), (63, 48), (52, 24), w, h, hill),
        Op::rect(0, 48, 64, 16, w, h, grass),
        Op::rect(16, 32, 19, 16, w, h, wall),
        Op::tri((13, 33), (38, 33), (25, 20), w, h, roof),
        Op::rect(23, 40, 5, 8, w, h, door),
        Op::tri((51, 4), (58, 11), (44, 11), w, h, sun),
        Op::tri((51, 18), (44, 11), (58, 11), w, h, sun),
    ]
}

// end{scene}

/// The traced run: a screen small enough that a waveform of the whole
/// render can be drawn and a testbench of it can be replayed, with
/// one of each kind of entry.
pub fn small(w: usize, h: usize) -> Vec<Op> {
    vec![
        Op::clear(w, h, 0x00_0020),
        Op::rect(2, 2, 5, 4, w, h, 0x40_8060),
        Op::tri((9, 13), (14, 13), (14, 5), w, h, 0xc0_4040),
    ]
}
