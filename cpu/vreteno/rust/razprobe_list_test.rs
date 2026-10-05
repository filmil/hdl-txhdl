// SPDX-License-Identifier: Apache-2.0
//! `razprobe`'s list (issue 1169), rendered on the host through
//! Razboj's model: every pixel the program checks on the board holds
//! there what the program expects.
mod razprobe_list;

use razboj::dl::decode;
use razboj::model::render;
use razprobe_list::{BACK, CHECKS, LIST};

#[test]
fn the_checks_hold_on_the_models_picture() {
    let ops: Vec<_> = LIST.iter().map(|w| decode(w)).collect();
    let fb = render(&ops, 1024, 480);
    for (x, y, want) in CHECKS {
        let got = fb[y as usize * 1024 + x as usize];
        assert_eq!(got, want, "pixel {x},{y}");
    }
}

/// Each shape covers a fair part of the screen and the backdrop the
/// rest, so the picture on the monitor is the one the board check
/// describes.
#[test]
fn every_shape_is_drawn() {
    let ops: Vec<_> = LIST.iter().map(|w| decode(w)).collect();
    let fb = render(&ops, 1024, 480);
    for (i, w) in LIST.iter().enumerate().skip(1) {
        let colour = w[0] >> 2;
        let n = (0..480)
            .flat_map(|y| (0..640).map(move |x| (x, y)))
            .filter(|&(x, y)| fb[y * 1024 + x] == colour)
            .count();
        assert!(n > 10_000, "shape {i} covers {n} pixels");
    }
    assert_eq!(fb[0], BACK);
}
