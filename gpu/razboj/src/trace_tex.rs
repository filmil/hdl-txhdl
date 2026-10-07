// SPDX-License-Identifier: Apache-2.0
//! The traced run of texturing (issue 997): in one tile, a quad of two
//! triangles in perspective, textured with an eight by eight texture
//! under `MODULATE`, repeated past its edges. It writes the trace and
//! the rasteriser's netlist, so the build replays the netlist under nvc
//! and Verilator against this run: the slots and the descriptor read with
//! the entry, the reciprocal, the texel's address, the cache's refills
//! and its hits, and the environment. It checks the picture against the
//! model.
use razboj::model::{self, Textures};
use razboj::op::{assemble, Op, TexMode};
use razboj::raster::Raster;
use razboj::sim::{self, Work};
use razboj_tile::tex::{encode, texel_offset, Desc, MODULATE, NEAREST};

/// One tile across, and forty rows of it.
const LOGW: usize = 6;
const W: usize = 1 << LOGW;
const H: usize = 40;
/// A link past sixteen bits, as the board's is.
const A: usize = 20;
/// Words of memory: the framebuffer, the texture, then the tile table
/// and its entries, and the count last.
const N: usize = 4096;
const DL: usize = 0x2800;
const CTRL: usize = 0x3ffc;
/// The texture's descriptor and its texels, between the framebuffer and
/// the list.
const DESC: u32 = 0x2600;
const BASE: u32 = 0x2640;

fn main() {
    let d = Desc {
        base: BASE,
        log_w: 3,
        log_h: 3,
        levels: 1,
        min: NEAREST,
        mag: NEAREST,
        ..Desc::default()
    };
    let mut more: Vec<(usize, u32)> = encode(&d)
        .iter()
        .enumerate()
        .map(|(k, w)| (DESC as usize + 4 * k, *w))
        .collect();
    for j in 0..8u32 {
        for i in 0..8u32 {
            let checker = if (i ^ j) & 1 == 1 { 0xff } else { 0x40 };
            let texel =
                0xff00_0000 | (checker << 16) | ((i * 32) << 8) | (j * 32);
            more.push(((BASE + texel_offset(&d, 0, i, j)) as usize, texel));
        }
    }
    // A floor: the near edge along the bottom, the far edge narrower
    // and higher, with the clip w at each corner and the texture twice
    // across.
    let corners = [
        ((2 * 16, 38 * 16), 1.0, (0.0, 0.0)),
        ((62 * 16, 38 * 16), 1.0, (16.0, 0.0)),
        ((44 * 16, 4 * 16), 3.0, (16.0, 16.0)),
        ((20 * 16, 4 * 16), 3.0, (0.0, 16.0)),
    ];
    let uvq = |k: usize| {
        let (_, w, (u, v)) = corners[k];
        let q = 1.0 / w;
        (
            (u * q * (1u64 << 32) as f64) as i64,
            (v * q * (1u64 << 32) as f64) as i64,
            (q * (1u64 << 48) as f64) as u64,
        )
    };
    let tri = |a: usize, b: usize, c: usize| Op::TexTri {
        a: corners[a].0,
        b: corners[b].0,
        c: corners[c].0,
        colours: [0xffc0_e0ff; 3],
        shaded: false,
        z: [0; 3],
        uvq: [uvq(a), uvq(b), uvq(c)],
    };
    let ops = [
        Op::Clear {
            colour: 0xff10_1820,
        },
        Op::Texture(Some(TexMode {
            desc: DESC,
            env: MODULATE,
            env_colour: 0,
        })),
        tri(0, 1, 2),
        tri(0, 2, 3),
    ];
    let list = assemble(&ops, W, H);
    let work = Work {
        more: more.clone(),
        ..Work::tiled(&list, W, H)
    };
    let runs = sim::run_works_at::<A, LOGW, H, N, DL, CTRL>(
        std::slice::from_ref(&work),
        true,
        false,
    );
    let r =
        Raster::<A, { sim::IDB }, LOGW, H, 0, DL, CTRL>::lowered("raster_tex");
    txhdl::netlist::write_netlists_from_env(&[&r]);
    let read = |a: u32| {
        more.iter()
            .find(|&&(at, _)| at as u32 == a)
            .map_or(0, |&(_, w)| w)
    };
    let t = Textures { mem: &read };
    let want = model::render_textured(&list, W, H, vec![0; W * H], Some(&t));
    assert_eq!(runs[0].fb, want, "texturing and the model differ");
    let bare = model::render(&list, W, H);
    let changed = want.iter().zip(&bare).filter(|(a, b)| a != b).count();
    assert!(changed > 500, "{changed} pixels textured");
    println!(
        "{} entries, {W} by {H} pixels, {changed} pixels textured, {} cycles, \
         {} read bursts",
        list.len(),
        runs[0].cycles,
        runs[0].reads.0
    );
}
