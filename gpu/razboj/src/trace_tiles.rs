// SPDX-License-Identifier: Apache-2.0
//! The traced run of a list drawn in tiles (issue 1255): a scene on a
//! screen two tiles across and one and an eighth down, binned into a
//! tile table and drawn through the tile buffer, then written out a row
//! at a time. It writes the trace and the rasteriser's netlist, so the
//! build replays the netlist under nvc and Verilator against this run,
//! the tile buffer and its write-out included, and it checks the picture
//! against the model.
use razboj::model;
use razboj::op::{assemble, Op};
use razboj::raster::Raster;
use razboj::sim::{self, Work};

/// Two tiles across, and a second row of tiles eight rows high: the
/// scene is as small as shows both, since nvc elaborates the whole run
/// within a heap of 16 MiB, and the scrub after reset is 4096 cycles.
const LOGW: usize = 7;
const W: usize = 1 << LOGW;
const H: usize = 72;
/// A link past sixteen bits, since the memory is 64 KiB.
const A: usize = 20;
/// Words of memory: the framebuffer, then the tile table and its
/// entries, and the count last.
const N: usize = 16384;
const DL: usize = 0xa000;
const CTRL: usize = 0xfffc;

fn main() {
    let ops = [
        Op::Clear { colour: 0x10_2030 },
        Op::Rect {
            colour: 0x12_3456,
            x: 50,
            y: 40,
            w: 40,
            h: 34,
        },
        Op::Gouraud {
            a: (20 * 16, 10 * 16 + 8),
            b: (120 * 16 + 4, 30 * 16),
            c: (60 * 16, 78 * 16 + 12),
            colours: [0xff_0000, 0x00_ff00, 0x00_00ff],
        },
    ];
    let insns = assemble(&ops, W, H);
    let work = Work::tiled(&insns, W, H);
    let tiles = work.count & 0xffff;
    let runs = sim::run_works_at::<A, LOGW, H, N, DL, CTRL>(
        std::slice::from_ref(&work),
        true,
        false,
    );
    let r = Raster::<A, { sim::IDB }, LOGW, H, 0, DL, CTRL>::lowered(
        "raster_tiles",
    );
    txhdl::netlist::write_netlists_from_env(&[&r]);
    let want = model::render(&insns, W, H);
    assert_eq!(runs[0].fb, want, "the tiled hardware and the model differ");
    println!(
        "{} entries in {tiles} tiles, {W} by {H} pixels, {} cycles",
        insns.len(),
        runs[0].cycles
    );
}
