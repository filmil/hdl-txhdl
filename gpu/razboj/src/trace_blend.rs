// SPDX-License-Identifier: Apache-2.0
//! The traced run of blending (issue 993): in one tile, a backdrop, then
//! a list without a clear, so that the tile is loaded from the
//! framebuffer first. The list's two triangles are blended over the
//! backdrop at half their alpha, one tests alpha, and the last writes
//! only some channels. It writes the trace and the rasteriser's netlist,
//! so the build replays the netlist under nvc and Verilator against this
//! run: the load, the colour bank's copy, the blend, the alpha test and
//! the mask. It checks the picture against the model.
use razboj::model;
use razboj::op::{assemble, AlphaTest, BlendMode, Op, GREATER};
use razboj::op::{ONE_MINUS_SRC_ALPHA, SRC_ALPHA};
use razboj::raster::Raster;
use razboj::sim::{self, Work};

/// One tile across, and forty rows of it.
const LOGW: usize = 6;
const W: usize = 1 << LOGW;
const H: usize = 40;
/// A link past sixteen bits, as the board's is.
const A: usize = 20;
/// Words of memory: the framebuffer, then the tile table and its
/// entries, and the count last.
const N: usize = 4096;
const DL: usize = 0x2800;
const CTRL: usize = 0x3ffc;

fn main() {
    let backdrop = [
        Op::Rect {
            colour: 0xff20_4080,
            x: 0,
            y: 0,
            w: 64,
            h: 20,
        },
        Op::Rect {
            colour: 0xff80_4020,
            x: 0,
            y: 20,
            w: 64,
            h: 20,
        },
    ];
    let ops = [
        Op::Blend(Some(BlendMode {
            src: SRC_ALPHA,
            dst: ONE_MINUS_SRC_ALPHA,
        })),
        Op::TriQ4 {
            colour: 0x80ff_ff00,
            a: (4 * 16, 4 * 16),
            b: (60 * 16, 8 * 16),
            c: (16 * 16, 38 * 16),
        },
        Op::AlphaTest(Some(AlphaTest {
            func: GREATER,
            reference: 0x80,
        })),
        Op::Gouraud {
            a: (56 * 16, 3 * 16),
            b: (50 * 16, 39 * 16),
            c: (6 * 16, 20 * 16),
            colours: [0xc000_ffff, 0xc0ff_00ff, 0xc0ff_ff00],
        },
        Op::Blend(None),
        Op::AlphaTest(None),
        Op::ColourMask(0b0100),
        Op::Rect {
            colour: 0xffff_ffff,
            x: 24,
            y: 12,
            w: 16,
            h: 16,
        },
    ];
    let (first, list) = (assemble(&backdrop, W, H), assemble(&ops, W, H));
    let works = [Work::tiled(&first, W, H), Work::tiled(&list, W, H)];
    assert!(
        works[1].words[1] & razboj_tile::LOAD != 0,
        "the tile is loaded"
    );
    let runs =
        sim::run_works_at::<A, LOGW, H, N, DL, CTRL>(&works, true, false);
    let r = Raster::<A, { sim::IDB }, LOGW, H, 0, DL, CTRL>::lowered(
        "raster_blend",
    );
    txhdl::netlist::write_netlists_from_env(&[&r]);
    let base = model::render(&first, W, H);
    let want = model::render_over(&list, W, H, base.clone());
    assert_eq!(runs[1].fb, want, "blending and the model differ");
    let changed = want.iter().zip(&base).filter(|(a, b)| a != b).count();
    assert!(changed > 500, "{changed} pixels blended");
    println!(
        "{} entries, {W} by {H} pixels, {changed} pixels changed, {} cycles",
        list.len(),
        runs[1].cycles
    );
}
