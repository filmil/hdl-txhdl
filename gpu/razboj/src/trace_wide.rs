// SPDX-License-Identifier: Apache-2.0
//! The traced run at the board's width (issue 1178): rows of 1024
//! pixels on a twenty-bit link, with something drawn past row 16,
//! where a pixel's offset no longer fits sixteen bits. It writes the
//! trace and the rasteriser's netlist, so the build replays the netlist
//! under nvc and Verilator against this run at that width, which the
//! sixteen-pixel trace cannot reach.
use razboj::model;
use razboj::op::{assemble, Op};
use razboj::raster::Raster;
use razboj::sim;

/// The board's row, and enough rows to pass the sixteenth.
const LOGW: usize = 10;
const W: usize = 1 << LOGW;
const H: usize = 24;
/// The link's address width: past sixteen bits, as the board's is.
const A: usize = 20;
/// Words of memory, 128 KiB: the framebuffer, then the list and its
/// count past it.
const N: usize = 32768;
const DL: usize = 0x1_8000;
const CTRL: usize = 0x1_c000;

fn main() {
    let ops = [
        Op::Rect {
            colour: 0x12_3456,
            x: 1000,
            y: 14,
            w: 4,
            h: 4,
        },
        Op::Tri {
            colour: 0x65_4321,
            a: (10, 18),
            b: (16, 23),
            c: (4, 23),
        },
    ];
    let insns = assemble(&ops, W, H);
    let runs = sim::run_lists_at::<A, LOGW, H, N, DL, CTRL>(
        std::slice::from_ref(&insns),
        true,
        false,
    );
    // The rasteriser's netlist under a name of its own, since the
    // sixteen-pixel trace's is `raster` and the two are checked side by
    // side.
    let r =
        Raster::<A, { sim::IDB }, LOGW, H, 0, DL, CTRL>::lowered("raster_wide");
    txhdl::netlist::write_netlists_from_env(&[&r]);
    let want = model::render(&insns, W, H);
    assert_eq!(runs[0].fb, want, "the hardware and the model differ");
    let drawn = want.iter().filter(|&&p| p != 0).count();
    println!(
        "{} entries, {W} by {H} pixels, {drawn} drawn, {} cycles",
        ops.len(),
        runs[0].cycles
    );
}
