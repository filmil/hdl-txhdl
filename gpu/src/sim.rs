// SPDX-License-Identifier: Apache-2.0
//! The whole design, run: the rasteriser, the two AXI trackers and
//! the framebuffer, joined and driven over a display list.
//!
//! The rasteriser is a host client written as hardware, so it holds
//! the link's channel ends themselves rather than a `Host`, which is
//! what [`axi_units`] hands out. Nothing else is between it and the
//! framebuffer but the five AXI channels.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, DefaultClock, Mem, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi_units, AxiHost, AxiPer, UnitLink};

use crate::fb::Fb;
use crate::op::{assemble, Insn, Op};
use crate::raster::Raster;

/// The address width of the link, and the identifiers: four of them,
/// so four writes are in flight at once.
pub const ADDR: usize = 16;
pub const IDB: usize = 2;
pub const IDS: usize = 4;

/// What a run reports: the framebuffer it left, and how many cycles
/// it took.
pub struct Run {
    pub fb: Vec<u32>,
    pub cycles: u64,
}

/// Render `ops` on the hardware. The screen is `1 << LOGW` by `H`
/// pixels and the framebuffer is `N` words, `N` a power of two at
/// least as large as the screen.
///
/// The display list is assembled here, since clipping and winding
/// are the host's work and not the rasteriser's, and then drawn as
/// [`run_list`] draws it.
///
/// `wave` writes the trace where `TXHDL_FST` says, and `netlists`
/// writes the two units' VHDL and Verilog where `TXHDL_VHDL` and
/// `TXHDL_VERILOG` say, so that the build can simulate the lowering
/// against this very run.
pub fn run<
    const LOGW: usize,
    const H: usize,
    const N: usize,
    const DL: usize,
    const CTRL: usize,
>(
    ops: &[Op],
    wave: bool,
    netlists: bool,
) -> Run {
    let insns = assemble(ops, 1usize << LOGW, H);
    run_list::<LOGW, H, N, DL, CTRL>(&insns, wave, netlists)
}

/// Render a display list that is already assembled, such as one a
/// program wrote. It is put in the memory at `DL` with its count at
/// `CTRL`, which is where the rasteriser goes looking for it, so the
/// run is the rasteriser alone on a link: what a system measures its
/// own run against.
pub fn run_list<
    const LOGW: usize,
    const H: usize,
    const N: usize,
    const DL: usize,
    const CTRL: usize,
>(
    insns: &[Insn],
    wave: bool,
    netlists: bool,
) -> Run {
    let w = 1usize << LOGW;
    // The memory the rasteriser reads its work out of and writes its
    // pixels into: the framebuffer at nought, the display list at
    // `DL`, and the count last, which is what says the list is ready.
    let mut image = vec![U::<32>::new(0); N];
    for (i, word) in crate::dl::image(insns).iter().enumerate() {
        image[DL / 4 + i] = U::from(*word);
    }
    image[CTRL / 4] = U::from(insns.len() as u32);
    let UnitLink {
        host_client,
        per_client,
        host_in,
        host_out,
        per_in,
        per_out,
    } = axi_units::<ADDR, 32, 4, IDB>();
    let (issue, wbeat, release, grant, done, rdata) = host_client;
    let (req, wd, ans, rb) = per_client;
    let (idle_out, idle) = signal::<Bit, DefaultClock>();

    let mut host = AxiHost::<ADDR, 32, 4, IDB, IDS>::default();
    let mut per = AxiPer::<ADDR, 32, 4, IDB>::default();
    let mut raster = Raster::<ADDR, IDB, LOGW, H, 0, DL, CTRL>::default();
    let mut fb = Fb::<ADDR, IDB, N> {
        px: Mem::with(&image),
        ..Default::default()
    };
    // The framebuffer is read out of the memory when the run has
    // finished, so a second handle on it is kept here.
    let pixels = fb.px.clone();

    if wave {
        if let Some(mut t) = Wave::from_env() {
            t.clock::<DefaultClock>();
            t.add("issue", &issue);
            t.add("wbeat", &wbeat);
            t.add("grant", &grant);
            t.add("done", &done);
            t.add("rdata", &rdata);
            t.add("release", &release);
            t.add("aw", &host_out.0);
            t.add("w", &host_out.2);
            t.add("b", &host_in.2);
            t.add("req", &per_out.0);
            t.add("wd", &per_out.1);
            t.add("ans", &per_in.3);
            t.add("rb", &per_in.4);
            t.add("idle", &idle);
            t.add("raster", &raster);
            t.add("fb", &fb);
            t.start();
        }
    }

    let start = now();
    let mut sim = Running::new(join2(
        join2(host.run(host_in, host_out), per.run(per_in, per_out)),
        join2(
            raster.run((grant, done, rdata), (issue, wbeat, release, idle_out)),
            fb.run((req, wd), (ans, rb)),
        ),
    ));
    // The rasteriser finds its own work, so the run only waits for it
    // to say it has nothing left to draw and nothing left in flight.
    let mut cycles = 0u64;
    let cap = 128 * N as u64 + 2000;
    loop {
        sim.cycle();
        cycles += 1;
        if idle.get().to_bool() {
            break;
        }
        assert!(cycles < cap, "the render did not finish in {cap} cycles");
    }
    if wave {
        stop();
    }
    if netlists {
        let r = Raster::<ADDR, IDB, LOGW, H, 0, DL, CTRL>::lowered("raster");
        // The memory starts with the display list in it, which the
        // lowering cannot see: `Mem::with` gave it at run time. The
        // netlist is told, or the fetch would read zeroes and the
        // simulated module would not follow the run it is checked
        // against. Only as far as the last word that says anything,
        // since the rest is the zero the array already starts at.
        let mut f = Fb::<ADDR, IDB, N>::lowered("fb");
        let last = image
            .iter()
            .rposition(|w| w.raw() != 0)
            .map_or(0, |i| i + 1);
        let words: Vec<u128> = image[..last].iter().map(|w| w.raw()).collect();
        f.init("px", &words);
        txhdl::netlist::write_netlists_from_env(&[&r, &f]);
    }
    let _ = start;
    Run {
        fb: (0..w * H).map(|i| pixels.read(i).raw() as u32).collect(),
        cycles,
    }
}

/// The hardware against the rule written with loops: every scene
/// rendered on the rasteriser, through the AXI link and into the
/// framebuffer, must leave exactly what the model leaves.
#[cfg(test)]
mod tests {
    use super::run;
    use crate::model;
    use crate::op::{assemble, Kind, Op};
    use crate::scene;

    /// The screen the tests use: sixteen by sixteen.
    const LOGW: usize = 4;
    const W: usize = 1 << LOGW;
    const H: usize = 16;
    /// Words of memory: the framebuffer, then the display list at
    /// [`DL`] and its count at [`CTRL`], and a power of two.
    const N: usize = 1024;
    /// Where the display list sits, clear of the framebuffer.
    const DL: usize = 0x400;
    /// Where the count sits, clear of the longest list a test makes.
    const CTRL: usize = 0x600;

    /// Render `ops` both ways and say where they differ.
    fn agree(ops: &[Op], what: &str) {
        let got = run::<LOGW, H, N, DL, CTRL>(ops, false, false);
        let want = model::render(&assemble(ops, W, H), W, H);
        for y in 0..H {
            for x in 0..W {
                assert_eq!(
                    got.fb[y * W + x],
                    want[y * W + x],
                    "{what}: pixel {x},{y}"
                );
            }
        }
    }

    /// A full-screen clear, as every test starts with.
    fn bg(colour: u32) -> Op {
        Op::Clear { colour }
    }

    #[test]
    fn a_scene_of_every_kind_agrees_with_the_model() {
        agree(&scene::small(), "the small scene");
    }

    /// The clear is the one entry whose box the hardware supplies.
    /// The instruction carries none, and the screen is filled anyway.
    #[test]
    fn a_clear_says_only_its_colour() {
        let insn = Op::Clear { colour: 0x31_41_59 }
            .encode(W, H)
            .expect("a clear always draws");
        assert_eq!(insn.kind, Kind::Clear);
        assert_eq!(insn.x1.raw(), 0, "a clear carries no box");
        assert_eq!(insn.y1.raw(), 0, "a clear carries no box");
        assert_eq!(insn.ax.raw(), 0, "a clear carries no vertices");
        let got = run::<LOGW, H, N, DL, CTRL>(&[bg(0x31_41_59)], false, false);
        assert!(
            got.fb.iter().all(|&p| p == 0x31_41_59),
            "the clear did not reach every pixel"
        );
    }

    #[test]
    fn a_triangle_is_the_same_whichever_way_it_is_wound() {
        let (a, b, c) = ((2, 2), (13, 5), (6, 14));
        let one = Op::Tri {
            colour: 0x00_ff00,
            a,
            b,
            c,
        };
        let other = Op::Tri {
            colour: 0x00_ff00,
            a,
            b: c,
            c: b,
        };
        agree(&[bg(0x10_1010), one], "one winding");
        agree(&[bg(0x10_1010), other], "the other winding");
        assert_eq!(
            model::render(&assemble(&[bg(0x10_1010), one], W, H), W, H),
            model::render(&assemble(&[bg(0x10_1010), other], W, H), W, H),
            "the two windings drew different pixels"
        );
    }

    #[test]
    fn a_triangle_hanging_off_the_screen_is_clipped() {
        let t = Op::Tri {
            colour: 0xff_0000,
            a: (-6, -6),
            b: (10, 2),
            c: (2, 10),
        };
        agree(&[bg(0), t], "a clipped triangle");
        let gone = Op::Tri {
            colour: 1,
            a: (-9, -9),
            b: (-4, -3),
            c: (-3, -4),
        };
        assert!(
            gone.encode(W, H).is_none(),
            "a triangle wholly off the screen is not an instruction"
        );
    }

    #[test]
    fn a_single_pixel_and_an_empty_box() {
        let dot = Op::Rect {
            colour: 0xff_ffff,
            x: 7,
            y: 9,
            w: 1,
            h: 1,
        };
        agree(&[bg(0), dot], "one pixel");
        let gone = Op::Rect {
            colour: 1,
            x: 20,
            y: 20,
            w: 4,
            h: 4,
        };
        assert!(
            gone.encode(W, H).is_none(),
            "a rectangle wholly off the screen is not an instruction"
        );
    }

    /// Scenes of pseudorandom rectangles and triangles, from a literal
    /// seed, every one of them rendered both ways.
    #[test]
    fn pseudorandom_scenes_agree_with_the_model() {
        let mut x = 0x9e37_79b9u32;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        let mut triangles = 0;
        let mut drawn = 0;
        for scene_no in 0..12 {
            let mut ops = vec![bg(0x20_2020)];
            for _ in 0..4 {
                let r = next();
                let p = |k: u32| (((r >> k) & 31) as i32) - 8;
                let colour = r & 0xff_ffff;
                let op = if r & 0x8000_0000 == 0 {
                    Op::Rect {
                        colour,
                        x: p(0),
                        y: p(5),
                        w: ((r >> 10) & 15) as i32 + 1,
                        h: ((r >> 14) & 15) as i32 + 1,
                    }
                } else {
                    triangles += 1;
                    Op::Tri {
                        colour,
                        a: (p(0), p(5)),
                        b: (p(10), p(15)),
                        c: (p(20), p(25)),
                    }
                };
                // An entry that draws nothing is left out, which is
                // what the assembler does with it too.
                if op.encode(W, H).is_some() {
                    drawn += 1;
                    ops.push(op);
                }
            }
            agree(&ops, &format!("scene {scene_no}"));
        }
        assert!(triangles > 8, "too few triangles drawn: {triangles}");
        assert!(drawn > 20, "too few entries drawn: {drawn}");
    }
}
