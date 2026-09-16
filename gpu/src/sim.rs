// SPDX-License-Identifier: Apache-2.0
//! The whole design, run: the rasteriser, the two AXI trackers and
//! the framebuffer, joined and driven over a display list.
//!
//! The rasteriser is a host client written as hardware, so it holds
//! the link's channel ends themselves rather than a `Host`, which is
//! what [`axi_units`] hands out. Nothing else is between it and the
//! framebuffer but the five AXI channels.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, join2, now, signal, DefaultClock, Running, Unit};
use txhdl::types::Bit;
use txhdl_parts::bus::axi::{axi_units, AxiHost, AxiPer, UnitLink};

use crate::fb::Fb;
use crate::op::Op;
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
/// `wave` writes the trace where `TXHDL_FST` says, and `netlists`
/// writes the two units' VHDL and Verilog where `TXHDL_VHDL` and
/// `TXHDL_VERILOG` say, so that the build can simulate the lowering
/// against this very run.
pub fn run<const LOGW: usize, const H: usize, const N: usize>(
    ops: &[Op],
    wave: bool,
    netlists: bool,
) -> Run {
    let w = 1usize << LOGW;
    let UnitLink {
        host_client,
        per_client,
        host_in,
        host_out,
        per_in,
        per_out,
    } = axi_units::<ADDR, 32, 4, IDB>();
    let (issue, wbeat, release, grant, done, _rdata) = host_client;
    let (req, wd, ans, rb) = per_client;
    let (op_tx, op_rx) = chan::<Op, DefaultClock>();
    let (idle_out, idle) = signal::<Bit, DefaultClock>();

    let mut host = AxiHost::<ADDR, 32, 4, IDB, IDS>::default();
    let mut per = AxiPer::<ADDR, 32, 4, IDB>::default();
    let mut raster = Raster::<ADDR, IDB, LOGW>::default();
    let mut fb = Fb::<ADDR, IDB, N>::default();
    // The framebuffer is read out of the memory when the run has
    // finished, so a second handle on it is kept here.
    let pixels = fb.px.clone();

    if wave {
        if let Some(mut t) = Wave::from_env() {
            t.clock::<DefaultClock>();
            t.add("ops", &op_rx);
            t.add("issue", &issue);
            t.add("wbeat", &wbeat);
            t.add("grant", &grant);
            t.add("done", &done);
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
            raster.run((op_rx, grant, done), (issue, wbeat, release, idle_out)),
            fb.run((req, wd), (ans, rb)),
        ),
    ));
    // Offer the display list as fast as the rasteriser takes it, then
    // wait for it to say it has nothing left to draw and nothing left
    // in flight.
    let mut next = 0;
    let mut cycles = 0u64;
    let cap = 64 * N as u64 + 1000;
    loop {
        if next < ops.len() && op_tx.ready().to_bool() {
            op_tx.send(ops[next]);
            next += 1;
        }
        sim.cycle();
        cycles += 1;
        if next == ops.len() && idle.get().to_bool() {
            break;
        }
        assert!(cycles < cap, "the render did not finish in {cap} cycles");
    }
    if wave {
        stop();
    }
    if netlists {
        let r = Raster::<ADDR, IDB, LOGW>::lowered("raster");
        let f = Fb::<ADDR, IDB, N>::lowered("fb");
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
    use crate::op::Op;
    use crate::scene;

    /// The screen the tests use: sixteen by sixteen.
    const LOGW: usize = 4;
    const W: usize = 1 << LOGW;
    const H: usize = 16;
    const N: usize = 256;

    /// Render `ops` both ways and say where they differ.
    fn agree(ops: &[Op], what: &str) {
        let got = run::<LOGW, H, N>(ops, false, false);
        let want = model::render(ops, W, H);
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

    #[test]
    fn a_scene_of_every_kind_agrees_with_the_model() {
        agree(&scene::small(W, H), "the small scene");
    }

    #[test]
    fn a_triangle_is_the_same_whichever_way_it_is_wound() {
        let a = (2, 2);
        let b = (13, 5);
        let c = (6, 14);
        let one = Op::tri(a, b, c, W, H, 0x00_ff00);
        let other = Op::tri(a, c, b, W, H, 0x00_ff00);
        let bg = Op::clear(W, H, 0x10_1010);
        agree(&[bg, one], "one winding");
        agree(&[bg, other], "the other winding");
        assert_eq!(
            model::render(&[bg, one], W, H),
            model::render(&[bg, other], W, H),
            "the two windings drew different pixels"
        );
    }

    #[test]
    fn a_triangle_hanging_off_the_screen_is_clipped() {
        let bg = Op::clear(W, H, 0);
        let t = Op::tri_checked((-6, -6), (10, 2), (2, 10), W, H, 0xff_0000)
            .expect("a triangle that is partly on the screen");
        agree(&[bg, t], "a clipped triangle");
        assert!(
            Op::tri_checked((-9, -9), (-4, -3), (-3, -4), W, H, 1).is_none(),
            "a triangle wholly off the screen is not an entry"
        );
    }

    #[test]
    fn a_single_pixel_and_an_empty_box() {
        let bg = Op::clear(W, H, 0);
        let dot = Op::rect(7, 9, 1, 1, W, H, 0xff_ffff);
        agree(&[bg, dot], "one pixel");
        assert!(
            Op::rect_checked(20, 20, 4, 4, W, H, 1).is_none(),
            "a rectangle wholly off the screen is not an entry"
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
        for scene_no in 0..12 {
            let mut ops = vec![Op::clear(W, H, 0x20_2020)];
            for _ in 0..4 {
                let r = next();
                let p = |k: u32| (((r >> k) & 31) as i32) - 8;
                let colour = r & 0xff_ffff;
                if r & 0x8000_0000 == 0 {
                    if let Some(o) = Op::rect_checked(
                        p(0),
                        p(5),
                        ((r >> 10) & 15) as i32 + 1,
                        ((r >> 14) & 15) as i32 + 1,
                        W,
                        H,
                        colour,
                    ) {
                        ops.push(o);
                    }
                } else if let Some(o) = Op::tri_checked(
                    (p(0), p(5)),
                    (p(10), p(15)),
                    (p(20), p(25)),
                    W,
                    H,
                    colour,
                ) {
                    triangles += 1;
                    ops.push(o);
                }
            }
            agree(&ops, &format!("scene {scene_no}"));
        }
        assert!(triangles > 8, "too few triangles drawn: {triangles}");
    }
}
