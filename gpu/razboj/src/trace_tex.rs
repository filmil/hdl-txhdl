// SPDX-License-Identifier: Apache-2.0
//! The traced run of texturing (issue 997): in one tile, a floor of two
//! triangles in perspective, textured with a sixteen by sixteen texture
//! of five levels under `BLEND`, repeated past its edges and filtered
//! with `LINEAR_MIPMAP_LINEAR`, magnified with `LINEAR`. It writes the
//! trace and the rasteriser's netlist, so the build replays the netlist
//! under nvc and Verilator against this run: the slots and the
//! descriptor read with the entry, the reciprocal, the level of detail,
//! each texel's address, the cache's refills and its hits, the filters
//! and the environment. It checks the picture against the model.
use razboj::model::{self, Textures};
use razboj::op::{assemble, Op, TexMode};
use razboj::raster::Raster;
use razboj::sim::{self, Work};
use razboj_tile::tex::{
    encode, level_base, side, texel_offset, Desc, BLEND, LINEAR,
    LINEAR_MIPMAP_LINEAR,
};

/// One tile across, and sixteen rows of it: the floor is small, since
/// a pixel here reads up to eight texels and the testbench holds every
/// cycle.
const LOGW: usize = 6;
const W: usize = 1 << LOGW;
const H: usize = 16;
/// A link past sixteen bits, as the board's is.
const A: usize = 20;
/// Words of memory: the framebuffer, the texture, then the tile table
/// and its entries, and the count last.
const N: usize = 4096;
const DL: usize = 0x2800;
const CTRL: usize = 0x3ffc;
/// The texture's descriptor and its levels, between the framebuffer and
/// the list.
const DESC: u32 = 0x1000;
const BASE: u32 = 0x1040;

fn main() {
    let d = Desc {
        base: BASE,
        log_w: 4,
        log_h: 4,
        levels: 5,
        min: LINEAR_MIPMAP_LINEAR,
        mag: LINEAR,
        ..Desc::default()
    };
    let mut more: Vec<(usize, u32)> = encode(&d)
        .iter()
        .enumerate()
        .map(|(k, w)| (DESC as usize + 4 * k, *w))
        .collect();
    // A checker, half as bright at each level down, so that the levels
    // show.
    for l in 0..d.levels {
        for j in 0..side(d.log_h, l) {
            for i in 0..side(d.log_w, l) {
                let checker = if (i ^ j) & 1 == 1 { 0xff } else { 0x40 };
                let c = checker >> l;
                let texel =
                    0xff00_0000 | (c << 16) | ((i * 16) << 8) | (j * 16);
                let at = level_base(&d, l) + texel_offset(&d, l, i, j);
                more.push((at as usize, texel));
            }
        }
    }
    // A floor: the near edge along the bottom, the far edge narrower
    // and higher, with the clip w at each corner and the texture four
    // times across.
    let corners = [
        ((16 * 16, 13 * 16), 1.0, (0.0, 0.0)),
        ((48 * 16, 13 * 16), 1.0, (64.0, 0.0)),
        ((38 * 16, 3 * 16), 4.0, (64.0, 64.0)),
        ((26 * 16, 3 * 16), 4.0, (0.0, 64.0)),
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
            env: BLEND,
            env_colour: 0xff20_80c0,
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
    assert!(changed > 150, "{changed} pixels textured");
    println!(
        "{} entries, {W} by {H} pixels, {changed} pixels textured, {} cycles, \
         {} read bursts",
        list.len(),
        runs[0].cycles,
        runs[0].reads.0
    );
}
