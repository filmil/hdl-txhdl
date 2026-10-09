// SPDX-License-Identifier: Apache-2.0
//! The traced run of the depth and the stencil on pixels the tile has not
//! written (issue 1534): in one tile and with no clear before it, a
//! triangle under `GL_LESS` whose stencil test, `GL_EQUAL` to nought,
//! reads every pixel as fresh, the farthest depth and a stencil of
//! nought, and counts the stencil up; then a second triangle that
//! crosses it in depth and draws only where the stencil is one. It
//! writes the trace and the rasteriser's netlist, so the build replays
//! the netlist under nvc and Verilator against this run: the mark's
//! choice of the fresh value on both tests, which `trace_depth`'s clear
//! of depth never asks for.
use razboj::model;
use razboj::op::stencil::{INCR, KEEP};
use razboj::op::{assemble, DepthMode, Op, StencilMode, EQUAL, LESS};
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

/// The stencil tested `GL_EQUAL` to `reference`, counted up where the
/// pixel passes both tests.
fn equal(reference: u32) -> Op {
    Op::Stencil(Some(StencilMode {
        func: EQUAL,
        reference,
        mask: 0xff,
        write_mask: 0xff,
        fail: KEEP,
        zfail: KEEP,
        zpass: INCR,
    }))
}

fn main() {
    let ops = [
        Op::Depth(Some(DepthMode {
            func: LESS,
            write: true,
        })),
        equal(0),
        Op::TriZ {
            colour: 0xc0_4000,
            a: (4 * 16, 4 * 16),
            b: (60 * 16, 8 * 16),
            c: (16 * 16, 38 * 16),
            z: [0x1000, 0xf000, 0x8000],
        },
        equal(1),
        Op::GouraudZ {
            a: (56 * 16, 3 * 16),
            b: (50 * 16, 39 * 16),
            c: (6 * 16, 20 * 16),
            colours: [0xff_ff00, 0x00_ffff, 0xff_00ff],
            z: [0x2000, 0x3000, 0xe000],
        },
    ];
    let insns = assemble(&ops, W, H);
    let work = Work::tiled(&insns, W, H);
    let runs = sim::run_works_at::<A, LOGW, H, N, DL, CTRL>(
        std::slice::from_ref(&work),
        true,
        false,
    );
    let r = Raster::<A, { sim::IDB }, LOGW, H, 0, DL, CTRL>::lowered(
        "raster_fresh",
    );
    txhdl::netlist::write_netlists_from_env(&[&r]);
    let want = model::render(&insns, W, H);
    assert_eq!(runs[0].fb, want, "the fresh tile and the model differ");
    let flat = want.iter().filter(|&&p| p == 0x00c0_4000).count();
    let shaded = want.iter().filter(|&&p| p != 0x00c0_4000 && p != 0).count();
    assert!(flat > 0 && shaded > 0, "each triangle drawn somewhere");
    println!(
        "{} entries, {W} by {H} pixels, flat at {flat}, shaded at {shaded}, \
         {} cycles",
        insns.len(),
        runs[0].cycles
    );
}
