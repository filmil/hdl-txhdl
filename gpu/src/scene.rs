// SPDX-License-Identifier: Apache-2.0
//! The scenes the demonstration and the traced run draw, as display
//! lists a program writes rather than as the instructions they encode
//! to.
use crate::op::Op;

// begin{scene}
/// The demonstration: a house under a hill, with a sun of two
/// triangles.
pub fn house() -> Vec<Op> {
    let sky = 0x10_1830;
    let hill = 0x37_474f;
    let grass = 0x2e_7d32;
    let wall = 0x8d_6e63;
    let roof = 0xc6_2828;
    let door = 0x4e_342e;
    let sun = 0xfd_d835;
    vec![
        Op::Clear { colour: sky },
        Op::Tri {
            colour: hill,
            a: (38, 48),
            b: (63, 48),
            c: (52, 24),
        },
        Op::Rect {
            colour: grass,
            x: 0,
            y: 48,
            w: 64,
            h: 16,
        },
        Op::Rect {
            colour: wall,
            x: 16,
            y: 32,
            w: 19,
            h: 16,
        },
        Op::Tri {
            colour: roof,
            a: (13, 33),
            b: (38, 33),
            c: (25, 20),
        },
        Op::Rect {
            colour: door,
            x: 23,
            y: 40,
            w: 5,
            h: 8,
        },
        Op::Tri {
            colour: sun,
            a: (51, 4),
            b: (58, 11),
            c: (44, 11),
        },
        Op::Tri {
            colour: sun,
            a: (51, 18),
            b: (44, 11),
            c: (58, 11),
        },
    ]
}
// end{scene}

/// The traced run: a screen small enough that a waveform of the whole
/// render can be drawn and a testbench of it can be replayed, with
/// one of each kind of entry.
pub fn small() -> Vec<Op> {
    vec![
        Op::Clear { colour: 0x00_0020 },
        Op::Rect {
            colour: 0x40_8060,
            x: 2,
            y: 2,
            w: 5,
            h: 4,
        },
        Op::Tri {
            colour: 0xc0_4040,
            a: (9, 13),
            b: (14, 13),
            c: (14, 5),
        },
    ]
}
