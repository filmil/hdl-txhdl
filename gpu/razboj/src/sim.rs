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
use txhdl_parts::bus::axi::{axi_units, AxiHost, AxiPer, PerPort, UnitLink};

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
    /// Read bursts the memory took and read beats it sent, until then.
    pub reads: (u64, u64),
    /// Write beats the memory took, until then: one a pixel written,
    /// and one for each count written back.
    pub writes: u64,
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
    let mut runs =
        run_lists::<LOGW, H, N, DL, CTRL>(&[insns.to_vec()], wave, netlists);
    runs.pop().expect("one list, one run")
}

/// Render several display lists one after another, as a program
/// drawing frame after frame does: the first is in the memory from
/// the start, and each of the others is written once the rasteriser
/// says the one before is done, a word a cycle with its count last,
/// which is what a program on the same memory would do. A [`Run`] per
/// list: the framebuffer as that list left it, and the cycles until
/// then.
///
/// With one list both units' netlists are written, and the build
/// checks both against this run. With more, only the rasteriser's
/// are, since the lists after the first reach the framebuffer's memory
/// from here and not through its port, which its netlist cannot see.
pub fn run_lists<
    const LOGW: usize,
    const H: usize,
    const N: usize,
    const DL: usize,
    const CTRL: usize,
>(
    lists: &[Vec<Insn>],
    wave: bool,
    netlists: bool,
) -> Vec<Run> {
    run_lists_at::<ADDR, LOGW, H, N, DL, CTRL>(lists, wave, netlists)
}

/// The same on a link of `A` address bits rather than [`ADDR`], for a
/// framebuffer past what sixteen bits reach: the board's is rows of
/// 1024 words (issue 1178).
pub fn run_lists_at<
    const A: usize,
    const LOGW: usize,
    const H: usize,
    const N: usize,
    const DL: usize,
    const CTRL: usize,
>(
    lists: &[Vec<Insn>],
    wave: bool,
    netlists: bool,
) -> Vec<Run> {
    let insns: &[Insn] = &lists[0];
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
    } = axi_units::<A, 32, 4, IDB>();
    let (issue, wbeat, release, grant, done, rdata) = host_client;
    let bus = PerPort::from(per_client);
    let (idle_out, idle) = signal::<Bit, DefaultClock>();
    // Nothing rings here: the count is read back to back.
    let (ring_out, ring) = signal::<Bit, DefaultClock>();
    ring_out.set(Bit::One);

    let mut host = AxiHost::<A, 32, 4, IDB, IDS>::default();
    let mut per = AxiPer::<A, 32, 4, IDB>::default();
    let mut raster = Raster::<A, IDB, LOGW, H, 0, DL, CTRL>::default();
    let mut fb = Fb::<A, IDB, N> {
        px: Mem::with(&image),
        ..Default::default()
    };
    // The framebuffer is read out of the memory when the run has
    // finished, so a second handle on it is kept here, and on the
    // counts of what was read.
    let pixels = fb.px.clone();
    let (rbursts, rbeats, wbeats) = (fb.rbursts, fb.rbeats, fb.wbeats);

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
            t.add("ring", &ring);
            t.add("raster", &raster);
            t.add("fb", &fb);
            t.start();
        }
    }

    let start = now();
    let mut sim = Running::new(join2(
        join2(host.run(host_in, host_out), per.run(per_in, per_out)),
        join2(
            raster.run(
                (grant, done, rdata, ring),
                (issue, wbeat, release, idle_out),
            ),
            fb.run(bus, ()),
        ),
    ));
    // The rasteriser finds its own work, so the run only waits for it
    // to say a list is drawn: `idle` rising. Then the next list goes
    // in, and the run waits for `idle` to fall and rise again.
    let mut cycles = 0u64;
    let cap = (128 * N as u64 + 2000) * lists.len() as u64;
    let mut runs = Vec::new();
    let mut was_idle = false;
    let mut next = 1;
    loop {
        sim.cycle();
        cycles += 1;
        let now_idle = idle.get().to_bool();
        if now_idle && !was_idle {
            runs.push(Run {
                fb: (0..w * H).map(|i| pixels.read(i).raw() as u32).collect(),
                cycles,
                reads: (rbursts.get().raw() as u64, rbeats.get().raw() as u64),
                writes: wbeats.get().raw() as u64,
            });
            if next == lists.len() {
                break;
            }
            // The program: the next list, a word a cycle, since the
            // memory takes one write a cycle, and the count last.
            for (i, word) in crate::dl::image(&lists[next]).iter().enumerate() {
                pixels.write(DL / 4 + i, U::<32>::from(*word));
                sim.cycle();
                cycles += 1;
            }
            pixels.write(CTRL / 4, U::<32>::from(lists[next].len() as u32));
            next += 1;
        }
        was_idle = now_idle;
        assert!(cycles < cap, "the render did not finish in {cap} cycles");
    }
    if wave {
        stop();
    }
    if netlists && lists.len() > 1 {
        // A name of its own, since the one-list run's netlist is
        // `raster` and the two are checked side by side.
        let r = Raster::<A, IDB, LOGW, H, 0, DL, CTRL>::lowered("raster_lists");
        txhdl::netlist::write_netlists_from_env(&[&r]);
    } else if netlists {
        let r = Raster::<A, IDB, LOGW, H, 0, DL, CTRL>::lowered("raster");
        // The memory starts with the display list in it, which the
        // lowering cannot see: `Mem::with` gave it at run time. The
        // netlist is told, or the fetch would read zeroes and the
        // simulated module would not follow the run it is checked
        // against. Only as far as the last word that says anything,
        // since the rest is the zero the array already starts at.
        let mut f = Fb::<A, IDB, N>::lowered("fb");
        // The link's four channels are traced under the names the run
        // gives them, which the waveform names too; the netlist calls
        // them the bundle's.
        f.trace_as("bus_req", "req");
        f.trace_as("bus_w", "wd");
        f.trace_as("bus_ans", "ans");
        f.trace_as("bus_r", "rb");
        let last = image
            .iter()
            .rposition(|w| w.raw() != 0)
            .map_or(0, |i| i + 1);
        let words: Vec<u128> = image[..last].iter().map(|w| w.raw()).collect();
        f.init("px", &words);
        txhdl::netlist::write_netlists_from_env(&[&r, &f]);
    }
    let _ = start;
    runs
}

/// The hardware against the rule written with loops: every scene
/// rendered on the rasteriser, through the AXI link and into the
/// framebuffer, must leave exactly what the model leaves.
#[cfg(test)]
mod tests {
    use super::{run, run_lists};
    use crate::dl::image;
    use crate::model;
    use crate::op::{assemble, Kind, Op};
    use crate::scene;
    use std::collections::BTreeSet;

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

    /// Two lists drawn back to back, as two frames are: the second is
    /// drawn over what the first left, both pictures are the model's,
    /// and the rasteriser says it is done twice, writing the count back
    /// to zero each time (issue 982).
    #[test]
    fn two_lists_are_drawn_one_after_the_other() {
        let first = assemble(&scene::small(), W, H);
        let second = assemble(
            &[
                Op::Rect {
                    colour: 0x12_3456,
                    x: 2,
                    y: 3,
                    w: 6,
                    h: 5,
                },
                Op::Tri {
                    colour: 0xfe_dcba,
                    a: (9, 1),
                    b: (15, 12),
                    c: (4, 14),
                },
            ],
            W,
            H,
        );
        let runs = run_lists::<LOGW, H, N, DL, CTRL>(
            &[first.clone(), second.clone()],
            false,
            false,
        );
        assert_eq!(runs.len(), 2, "two lists, two done");
        assert_eq!(runs[0].fb, model::render(&first, W, H), "the first");
        let both: Vec<_> = first.iter().chain(&second).copied().collect();
        assert_eq!(runs[1].fb, model::render(&both, W, H), "the second");
        assert!(runs[1].cycles > runs[0].cycles);
    }

    /// A list longer than 255 entries, which an eight-bit count cut
    /// short: three hundred single pixels, the last forty-four over
    /// the first, in colours of their own (issue 983).
    #[test]
    fn a_list_of_three_hundred_entries_is_drawn_whole() {
        const N: usize = 8192;
        const DL: usize = 0x1000;
        const CTRL: usize = 0x7000;
        let ops: Vec<Op> = (0..300)
            .map(|i: i32| Op::Rect {
                colour: 0x01_0101 * (i as u32 % 200) + i as u32,
                x: i % 16,
                y: (i / 16) % 16,
                w: 1,
                h: 1,
            })
            .collect();
        let insns = assemble(&ops, W, H);
        assert_eq!(insns.len(), 300);
        let got = super::run::<LOGW, H, N, DL, CTRL>(&ops, false, false);
        assert_eq!(got.fb, model::render(&insns, W, H), "300 entries");
    }

    /// An entry is fetched as one read burst of sixteen beats, so every
    /// entry costs fifteen beats more than it costs bursts; a poll of
    /// the count is one beat and one burst, and costs neither
    /// (issue 983).
    #[test]
    fn an_entry_is_fetched_in_one_burst() {
        let ops = scene::small();
        let n = assemble(&ops, W, H).len() as u64;
        let got = run::<LOGW, H, N, DL, CTRL>(&ops, false, false);
        let (bursts, beats) = got.reads;
        assert_eq!(beats - bursts, 15 * n, "{bursts} bursts, {beats} beats");
    }

    /// A mesh of triangles sharing edges, over the whole screen: four
    /// by four cells of four pixels, each cut in two along a diagonal
    /// that runs through pixel centres, the inner vertices moved by
    /// sixteenths of a pixel. Every pixel is covered by exactly one
    /// triangle, in the model, and the hardware writes each exactly
    /// once and draws what the model draws (issue 988).
    #[test]
    fn a_mesh_draws_every_pixel_once() {
        let jitter = |i: i32, j: i32| -> (i32, i32) {
            if i == 0 || j == 0 || i == 4 || j == 4 {
                (0, 0)
            } else {
                ((i * 7 + j * 3) % 11 - 5, (i * 5 + j * 7) % 13 - 6)
            }
        };
        let v = |i: i32, j: i32| {
            let (dx, dy) = jitter(i, j);
            (64 * i + dx, 64 * j + dy)
        };
        let mut ops = vec![bg(0)];
        let mut colour = 0x10_0000;
        for j in 0..4 {
            for i in 0..4 {
                let (a, b, c, d) =
                    (v(i, j), v(i + 1, j), v(i + 1, j + 1), v(i, j + 1));
                for (p, q, r) in [(a, b, c), (a, c, d)] {
                    colour += 0x01_0203;
                    ops.push(Op::TriQ4 {
                        colour,
                        a: p,
                        b: q,
                        c: r,
                    });
                }
            }
        }
        let list = assemble(&ops, W, H);
        assert_eq!(list.len(), 33, "a clear and 32 triangles");
        let cover = model::coverage(&list[1..], W, H);
        assert!(
            cover.iter().all(|&n| n == 1),
            "every pixel covered once: {cover:?}"
        );
        // Thirty-three entries reach past the usual count's place, so
        // the memory is the larger one the long list uses.
        const N: usize = 4096;
        const DL: usize = 0x1000;
        const CTRL: usize = 0x3800;
        let got = run::<LOGW, H, N, DL, CTRL>(&ops, false, false);
        assert_eq!(got.fb, model::render(&list, W, H), "the picture");
        // The clear writes every pixel, the mesh every pixel once more,
        // and the count is written back to zero at the end.
        assert_eq!(got.writes, (2 * W * H + 1) as u64, "pixels written");
    }

    /// A shaded triangle is drawn as the model draws it: each pixel the
    /// three planes at its centre, wound either way, hanging off the
    /// screen, and under a flat triangle drawn over part of it
    /// (issue 989).
    #[test]
    fn a_shaded_triangle_agrees_with_the_model() {
        let colours = [0xff_8000, 0x10_ff40, 0x30_20ff];
        let shaded = |a, b, c| Op::Gouraud { a, b, c, colours };
        let (a, b, c) = ((24, 24), (232, 40), (56, 232));
        agree(&[bg(0x20_2020), shaded(a, b, c)], "shaded");
        agree(&[bg(0x20_2020), shaded(a, c, b)], "wound the other way");
        agree(
            &[bg(0), shaded((-80, -40), (400, 60), (90, 330))],
            "off the screen",
        );
        let flat = Op::TriQ4 {
            colour: 0xab_cdef,
            a: (100, 100),
            b: (250, 120),
            c: (140, 250),
        };
        agree(&[bg(0), shaded(a, b, c), flat], "under a flat one");
    }

    /// Every entry writes its alpha in the pixel's top byte: a clear's,
    /// a rectangle's, a flat triangle's and a shaded one's, the last
    /// its first vertex's, and no other (issue 990).
    #[test]
    fn alpha_is_written_in_the_pixels_top_byte() {
        let ops = [
            bg(0x1120_3040),
            Op::Rect {
                colour: 0x2240_5060,
                x: 1,
                y: 1,
                w: 5,
                h: 4,
            },
            Op::Tri {
                colour: 0x3370_8090,
                a: (8, 1),
                b: (15, 6),
                c: (9, 7),
            },
            Op::Gouraud {
                a: (24, 136),
                b: (120, 248),
                c: (8, 248),
                colours: [0x44ff_0000, 0x9900_ff00, 0xaa00_00ff],
            },
        ];
        agree(&ops, "alpha");
        let got = run::<LOGW, H, N, DL, CTRL>(&ops, false, false);
        let alphas: BTreeSet<u32> = got.fb.iter().map(|p| p >> 24).collect();
        assert_eq!(alphas, BTreeSet::from([0x11, 0x22, 0x33, 0x44]));
    }

    /// A scissor box: nothing outside it is touched, inside it the
    /// picture is the model's, and a clear under it fills only the box.
    /// A box that holds the screen is no box at all, and one wholly off
    /// it draws nothing (issue 990).
    #[test]
    fn nothing_outside_the_scissor_box_is_touched() {
        let (x0, y0, x1, y1) = (3, 4, 11, 12);
        let scissor = Op::Scissor {
            x: x0,
            y: y0,
            w: x1 - x0 + 1,
            h: y1 - y0 + 1,
        };
        let drawn = [
            bg(0x20_4060),
            Op::Rect {
                colour: 0x80_8080,
                x: 0,
                y: 0,
                w: 6,
                h: 6,
            },
            Op::Tri {
                colour: 0xc0_4040,
                a: (-2, 10),
                b: (18, 2),
                c: (8, 18),
            },
            Op::Gouraud {
                a: (0, 0),
                b: (256, 64),
                c: (64, 256),
                colours: [0xff_0000, 0x00_ff00, 0x00_00ff],
            },
        ];
        let with = |first: Op| {
            let mut ops = vec![bg(0x10_1010), first];
            ops.extend(drawn);
            ops
        };
        let ops = with(scissor);
        agree(&ops, "scissored");
        let got = run::<LOGW, H, N, DL, CTRL>(&ops, false, false);
        for y in 0..H as i32 {
            for x in 0..W as i32 {
                let inside = (x0..=x1).contains(&x) && (y0..=y1).contains(&y);
                if !inside {
                    let p = got.fb[y as usize * W + x as usize];
                    assert_eq!(p, 0x10_1010, "pixel {x},{y} outside");
                }
            }
        }
        let list = assemble(&ops, W, H);
        assert_eq!(list[1].kind, Kind::Rect, "a clear under a scissor box");
        // A box that holds the screen assembles as no box.
        let whole = Op::Scissor {
            x: -5,
            y: -5,
            w: 100,
            h: 100,
        };
        let mut plain = with(whole);
        plain.remove(1);
        assert_eq!(
            image(&assemble(&with(whole), W, H)),
            image(&assemble(&plain, W, H)),
            "a scissor box the size of the screen"
        );
        // One off the screen leaves only the first clear.
        let gone = Op::Scissor {
            x: 20,
            y: 0,
            w: 4,
            h: 4,
        };
        assert_eq!(assemble(&with(gone), W, H).len(), 1, "nothing inside");
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
