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
    /// Write beats the memory took that wrote something, until then:
    /// one a pixel written, and one for each count written back.
    pub writes: u64,
    /// Write bursts the memory took, until then: one a run of pixels in
    /// a row, and one for each count written back (issue 987).
    pub bursts: u64,
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
    let works: Vec<Work> = lists.iter().map(|l| Work::flat(l)).collect();
    run_works_at::<A, LOGW, H, N, DL, CTRL>(&works, wave, netlists)
}

/// A list as the rasteriser finds it in memory: the words at `DL`, and
/// the count word at `CTRL` that says it is there; and words it reads
/// elsewhere, such as a texture's (issue 997), each at its byte address,
/// written before the count.
pub struct Work {
    pub words: Vec<u32>,
    pub count: u32,
    pub more: Vec<(usize, u32)>,
}

impl Work {
    /// A flat list, its entries one after another.
    pub fn flat(list: &[Insn]) -> Work {
        Work {
            words: crate::dl::image(list),
            count: list.len() as u32,
            more: Vec::new(),
        }
    }

    /// The same list binned into tiles on a screen of `sw` by `sh`, with
    /// the count word that says so (issue 1255).
    pub fn tiled(list: &[Insn], sw: usize, sh: usize) -> Work {
        let (words, count) = crate::tiles::image(list, sw, sh);
        Work {
            words,
            count,
            more: Vec::new(),
        }
    }
}

/// Render lists laid out in memory as [`Work`] says, flat or in tiles,
/// one after another, as [`run_lists_at`] does.
pub fn run_works_at<
    const A: usize,
    const LOGW: usize,
    const H: usize,
    const N: usize,
    const DL: usize,
    const CTRL: usize,
>(
    lists: &[Work],
    wave: bool,
    netlists: bool,
) -> Vec<Run> {
    let w = 1usize << LOGW;
    // The memory the rasteriser reads its work out of and writes its
    // pixels into: the framebuffer at nought, the display list at
    // `DL`, and the count last, which is what says the list is ready.
    let mut image = vec![U::<32>::new(0); N];
    for (i, word) in lists[0].words.iter().enumerate() {
        image[DL / 4 + i] = U::from(*word);
    }
    for (at, word) in &lists[0].more {
        image[at / 4] = U::from(*word);
    }
    image[CTRL / 4] = U::from(lists[0].count);
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
    let (rbursts, rbeats, wbeats, wbursts) =
        (fb.rbursts, fb.rbeats, fb.wbeats, fb.wbursts);

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
                bursts: wbursts.get().raw() as u64,
            });
            if next == lists.len() {
                break;
            }
            // The program: the next list, a word a cycle, since the
            // memory takes one write a cycle, and the count last.
            for (i, word) in lists[next].words.iter().enumerate() {
                pixels.write(DL / 4 + i, U::<32>::from(*word));
                sim.cycle();
                cycles += 1;
            }
            for (at, word) in &lists[next].more {
                pixels.write(at / 4, U::<32>::from(*word));
                sim.cycle();
                cycles += 1;
            }
            pixels.write(CTRL / 4, U::<32>::from(lists[next].count));
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
    use super::{run, run_lists, run_lists_at, run_works_at, Work};
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

    /// Depth in tiles (issue 992): scenes of overlapping flat and shaded
    /// triangles at depths across the range, after a clear of depth, under
    /// each of GL's eight comparisons, with depth written and not, and
    /// entries that do not test depth among them, are drawn in tiles as
    /// the model draws them, byte for byte. The same lists drawn flat are
    /// the model's with the depth taken out, which is the documented
    /// limit: a flat list has no depth.
    #[test]
    fn depth_in_tiles_is_the_models() {
        use crate::op::{DepthMode, ALWAYS};
        const A: usize = 20;
        const LOGW: usize = 7;
        const W: usize = 1 << LOGW;
        const H: usize = 80;
        const N: usize = 16384;
        const DL: usize = 0xa000;
        const CTRL: usize = 0xfffc;
        let mut x = 0x2468_ace1u32;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        let mut drawn_differently = 0;
        for func in 0..8u32 {
            let mut ops = vec![
                Op::Depth(Some(DepthMode {
                    func: ALWAYS,
                    write: true,
                })),
                Op::RectZ {
                    colour: 0x10_1010,
                    x: 0,
                    y: 0,
                    w: W as i32,
                    h: H as i32,
                    z: 0x8000,
                },
                Op::Depth(Some(DepthMode {
                    func,
                    write: func % 2 == 1,
                })),
            ];
            for k in 0..8 {
                let (r, q) = (next(), next());
                let p = |v: u32| {
                    (
                        (v % (W as u32 * 16)) as i32 - 64,
                        ((v >> 12) % (H as u32 * 16)) as i32 - 64,
                    )
                };
                let z = [r & 0xffff, q & 0xffff, (r >> 16) ^ (q >> 16)];
                let (a, b, c) = (p(r), p(q), p(r ^ q.rotate_left(7)));
                ops.push(match k % 3 {
                    0 => Op::TriZ {
                        colour: q & 0xff_ffff,
                        a,
                        b,
                        c,
                        z,
                    },
                    1 => Op::GouraudZ {
                        a,
                        b,
                        c,
                        colours: [q, r, q ^ r].map(|c| c & 0xff_ffff),
                        z,
                    },
                    // No depth: drawn over whatever is there.
                    _ => Op::TriQ4 {
                        colour: r & 0xff_ffff,
                        a,
                        b,
                        c,
                    },
                });
            }
            let list = assemble(&ops, W, H);
            assert!(list.iter().any(|i| i.depth.to_bool()), "func {func}");
            let want = model::render(&list, W, H);
            let tiled = run_works_at::<A, LOGW, H, N, DL, CTRL>(
                &[Work::tiled(&list, W, H)],
                false,
                false,
            );
            let at = want.iter().zip(&tiled[0].fb).position(|(p, q)| p != q);
            assert_eq!(at, None, "func {func}: the first pixel that differs");
            let flat = run_works_at::<A, LOGW, H, N, DL, CTRL>(
                &[Work::flat(&list)],
                false,
                false,
            );
            let mut plain = list.clone();
            for i in &mut plain {
                i.depth = txhdl::types::Bit::Zero;
            }
            assert_eq!(flat[0].fb, model::render(&plain, W, H), "func {func}");
            drawn_differently += (want != flat[0].fb) as u32;
        }
        assert!(
            drawn_differently >= 5,
            "depth mattered: {drawn_differently}"
        );
    }

    /// Scenes that blend, test alpha and mask channels (issue 993), with
    /// and without depth, are drawn in tiles as the model draws them,
    /// byte for byte, under every blend factor and comparison. Each scene
    /// is drawn twice: over the last picture without a clear, so that its
    /// tiles are loaded from the framebuffer first and the blend reads
    /// what memory had; and after a clear, which loads no tile.
    #[test]
    fn blending_in_tiles_is_the_models() {
        use crate::op::{AlphaTest, BlendMode, DepthMode, LESS};
        use razboj_tile::LOAD;
        const A: usize = 20;
        const LOGW: usize = 7;
        const W: usize = 1 << LOGW;
        const H: usize = 64;
        const N: usize = 16384;
        const DL: usize = 0xa000;
        const CTRL: usize = 0xfffc;
        let mut x = 0x9e37_79b9u32;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        // How many of a work's tiles are to be loaded.
        let loads = |w: &Work| {
            (0..(w.count & 0xffff) as usize)
                .filter(|t| w.words[2 * t + 1] & LOAD != 0)
                .count()
        };
        // A backdrop that blending reads: bands of colour and alpha.
        let backdrop: Vec<Op> = (0..4)
            .map(|k| Op::Rect {
                colour: 0x40_2010u32.wrapping_mul(k + 1) | (k * 0x3f) << 24,
                x: 0,
                y: k as i32 * 16,
                w: W as i32,
                h: 16,
            })
            .collect();
        let first = assemble(&backdrop, W, H);
        let mut changed = 0;
        for round in 0..11u32 {
            let mut ops = vec![
                Op::Blend(Some(BlendMode {
                    src: round,
                    dst: (round * 7 + 3) % 10,
                })),
                Op::AlphaTest((round % 3 == 1).then_some(AlphaTest {
                    func: round % 8,
                    reference: 0x60,
                })),
                Op::ColourMask(if round % 4 == 2 { 0b1011 } else { 0xf }),
                Op::Depth((round % 2 == 1).then_some(DepthMode {
                    func: LESS,
                    write: true,
                })),
            ];
            for k in 0..6 {
                let (r, q) = (next(), next());
                let p = |v: u32| {
                    (
                        (v % (W as u32 * 16)) as i32 - 64,
                        ((v >> 12) % (H as u32 * 16)) as i32 - 64,
                    )
                };
                let z = [r & 0xffff, q & 0xffff, (r >> 16) ^ (q >> 16)];
                let (a, b, c) = (p(r), p(q), p(r ^ q.rotate_left(7)));
                ops.push(match k % 3 {
                    0 => Op::TriZ {
                        colour: q,
                        a,
                        b,
                        c,
                        z,
                    },
                    1 => Op::GouraudZ {
                        a,
                        b,
                        c,
                        colours: [q, r, q ^ r],
                        z,
                    },
                    _ => Op::Rect {
                        colour: r,
                        x: (r % 100) as i32,
                        y: (q % 50) as i32,
                        w: 20,
                        h: 12,
                    },
                });
            }
            let over = assemble(&ops, W, H);
            let mut cleared = vec![Op::Clear {
                colour: 0x8020_4060,
            }];
            cleared.extend(ops.iter().copied());
            let fresh = assemble(&cleared, W, H);
            let (w1, w2, w3) = (
                Work::tiled(&first, W, H),
                Work::tiled(&over, W, H),
                Work::tiled(&fresh, W, H),
            );
            assert!(loads(&w2) > 0, "round {round}: a blend over memory loads");
            assert_eq!(loads(&w3), 0, "round {round}: after a clear, none");
            let runs = run_works_at::<A, LOGW, H, N, DL, CTRL>(
                &[w1, w2, w3],
                false,
                false,
            );
            let base = model::render(&first, W, H);
            assert_eq!(runs[0].fb, base, "round {round}: the backdrop");
            let want = model::render_over(&over, W, H, base.clone());
            let at = want.iter().zip(&runs[1].fb).position(|(p, q)| p != q);
            assert_eq!(at, None, "round {round}: blended over memory");
            let want = model::render_over(&fresh, W, H, want);
            let at = want.iter().zip(&runs[2].fb).position(|(p, q)| p != q);
            assert_eq!(at, None, "round {round}: blended after a clear");
            changed += (runs[1].fb
                != model::render_over(
                    &over
                        .iter()
                        .map(|i| {
                            let mut i = *i;
                            i.state = txhdl::types::Bit::Zero;
                            i
                        })
                        .collect::<Vec<_>>(),
                    W,
                    H,
                    base,
                )) as u32;
        }
        assert!(changed >= 8, "the state mattered in {changed} rounds");
    }

    /// Logic operations in tiles (issue 998): each of GL's sixteen, over
    /// a backdrop the tiles load from memory, with and without depth and
    /// a colour mask in some rounds, and in some with a blend set that the
    /// logic operation must take the place of, are drawn as the model draws
    /// them, byte for byte; and each but `GL_COPY` changes the picture. In
    /// the rounds with neither a blend nor a mask, the logic operation is
    /// what has the tiles loaded.
    #[test]
    fn logic_ops_in_tiles_are_the_models() {
        use crate::op::{BlendMode, DepthMode, COPY, LESS, ONE, SRC_ALPHA};
        const A: usize = 20;
        const LOGW: usize = 7;
        const W: usize = 1 << LOGW;
        const H: usize = 64;
        const N: usize = 16384;
        const DL: usize = 0xa000;
        const CTRL: usize = 0xfffc;
        let mut x = 0x1b87_3593u32;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        let backdrop: Vec<Op> = (0..4)
            .map(|k| Op::Rect {
                colour: 0x5a_a53cu32.wrapping_mul(k + 3) | (k * 0x4b) << 24,
                x: 0,
                y: k as i32 * 16,
                w: W as i32,
                h: 16,
            })
            .collect();
        let first = assemble(&backdrop, W, H);
        let mut changed = 0;
        for op in 0..16u32 {
            let mut ops = vec![
                Op::Blend((op % 4 == 0).then_some(BlendMode {
                    src: SRC_ALPHA,
                    dst: ONE,
                })),
                Op::LogicOp(Some(op)),
                Op::ColourMask(if op % 3 == 1 { 0b0111 } else { 0xf }),
                Op::Depth((op % 2 == 1).then_some(DepthMode {
                    func: LESS,
                    write: true,
                })),
            ];
            for k in 0..4 {
                let (r, q) = (next(), next());
                let p = |v: u32| {
                    (
                        (v % (W as u32 * 16)) as i32 - 64,
                        ((v >> 12) % (H as u32 * 16)) as i32 - 64,
                    )
                };
                let z = [r & 0xffff, q & 0xffff, (r >> 16) ^ (q >> 16)];
                let (a, b, c) = (p(r), p(q), p(r ^ q.rotate_left(7)));
                ops.push(if k % 2 == 0 {
                    Op::TriZ {
                        colour: q,
                        a,
                        b,
                        c,
                        z,
                    }
                } else {
                    Op::GouraudZ {
                        a,
                        b,
                        c,
                        colours: [q, r, q ^ r],
                        z,
                    }
                });
            }
            let over = assemble(&ops, W, H);
            let runs = run_works_at::<A, LOGW, H, N, DL, CTRL>(
                &[Work::tiled(&first, W, H), Work::tiled(&over, W, H)],
                false,
                false,
            );
            let base = model::render(&first, W, H);
            let want = model::render_over(&over, W, H, base.clone());
            let at = want.iter().zip(&runs[1].fb).position(|(p, q)| p != q);
            assert_eq!(
                at,
                None,
                "op {op}: {:08x} not {:08x}",
                at.map_or(0, |a| runs[1].fb[a]),
                at.map_or(0, |a| want[a])
            );
            // Against the same entries copying the pixel through.
            let copy: Vec<_> = over
                .iter()
                .map(|i| {
                    let mut i = *i;
                    i.lop = txhdl::types::U::from(COPY);
                    i
                })
                .collect();
            let copied = model::render_over(&copy, W, H, base);
            changed += (want != copied) as u32;
        }
        assert_eq!(changed, 15, "every op but GL_COPY changed the picture");
    }

    /// Fog in tiles (issue 998): flat and shaded triangles and rectangles,
    /// each with its own fog colour and factors, over a backdrop the tiles
    /// load, under a blend, an alpha test, a colour mask, depth and a
    /// scissor box in some rounds, and a clear that fog leaves alone, are
    /// drawn as the model draws them, byte for byte; and the fog changes
    /// the picture in every round.
    #[test]
    fn fog_in_tiles_is_the_models() {
        use crate::op::{AlphaTest, BlendMode, DepthMode, Fog, GREATER, LESS};
        use crate::op::{ONE_MINUS_SRC_ALPHA, SRC_ALPHA};
        const A: usize = 20;
        const LOGW: usize = 7;
        const W: usize = 1 << LOGW;
        const H: usize = 64;
        const N: usize = 16384;
        const DL: usize = 0xa000;
        const CTRL: usize = 0xfffc;
        let mut x = 0x6d2b_79f5u32;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        let backdrop: Vec<Op> = (0..4)
            .map(|k| Op::Rect {
                colour: 0x3c_a55au32.wrapping_mul(k + 5) | (k * 0x55) << 24,
                x: 0,
                y: k as i32 * 16,
                w: W as i32,
                h: 16,
            })
            .collect();
        let first = assemble(&backdrop, W, H);
        let mut changed = 0;
        for round in 0..12u32 {
            let mut ops = vec![
                Op::Blend((round % 3 == 1).then_some(BlendMode {
                    src: SRC_ALPHA,
                    dst: ONE_MINUS_SRC_ALPHA,
                })),
                Op::AlphaTest((round % 4 == 2).then_some(AlphaTest {
                    func: GREATER,
                    reference: 0x60,
                })),
                Op::ColourMask(if round % 5 == 3 { 0b1011 } else { 0xf }),
                Op::Depth((round % 2 == 1).then_some(DepthMode {
                    func: LESS,
                    write: true,
                })),
            ];
            if round % 6 == 4 {
                ops.push(Op::Scissor {
                    x: 20,
                    y: 8,
                    w: 70,
                    h: 40,
                });
            }
            if round % 4 == 0 {
                ops.push(Op::Fog(Some(Fog {
                    colour: next(),
                    f: [0, 0, 0],
                })));
                ops.push(Op::Clear {
                    colour: next() | 0x8000_0000,
                });
            }
            for k in 0..5 {
                let (r, q, s) = (next(), next(), next());
                ops.push(Op::Fog((k != 3).then_some(Fog {
                    colour: s,
                    f: [s >> 24, (s >> 16) & 0xff, (r >> 8) & 0xff],
                })));
                let p = |v: u32| {
                    (
                        (v % (W as u32 * 16)) as i32 - 64,
                        ((v >> 12) % (H as u32 * 16)) as i32 - 64,
                    )
                };
                let z = [r & 0xffff, q & 0xffff, (r >> 16) ^ (q >> 16)];
                let (a, b, c) = (p(r), p(q), p(r ^ q.rotate_left(7)));
                ops.push(match k {
                    0 | 3 => Op::TriZ {
                        colour: q,
                        a,
                        b,
                        c,
                        z,
                    },
                    1 => Op::GouraudZ {
                        a,
                        b,
                        c,
                        colours: [q, r, q ^ r],
                        z,
                    },
                    2 => Op::Gouraud {
                        a,
                        b,
                        c,
                        colours: [r, q ^ s, q],
                    },
                    _ => Op::Rect {
                        colour: q,
                        x: (r % 100) as i32,
                        y: (q % 40) as i32,
                        w: 30,
                        h: 20,
                    },
                });
            }
            let over = assemble(&ops, W, H);
            assert!(over.iter().any(|i| i.fog.to_bool()));
            let runs = run_works_at::<A, LOGW, H, N, DL, CTRL>(
                &[Work::tiled(&first, W, H), Work::tiled(&over, W, H)],
                false,
                false,
            );
            let base = model::render(&first, W, H);
            let want = model::render_over(&over, W, H, base.clone());
            let at = want.iter().zip(&runs[1].fb).position(|(p, q)| p != q);
            assert_eq!(
                at,
                None,
                "round {round}: {:08x} not {:08x}",
                at.map_or(0, |a| runs[1].fb[a]),
                at.map_or(0, |a| want[a])
            );
            // Against the same entries without their fog.
            let clear: Vec<_> = over
                .iter()
                .map(|i| {
                    let mut i = *i;
                    i.fog = txhdl::types::Bit::Zero;
                    i
                })
                .collect();
            changed += (model::render_over(&clear, W, H, base) != want) as u32;
        }
        assert_eq!(changed, 12, "the fog changed the picture in every round");
    }

    /// Texturing in tiles (issue 997): textured triangles in perspective,
    /// flat and shaded, under every class of texel, every environment,
    /// both magnification filters and all six minification filters over
    /// a texture of six levels, repeated and clamped, magnified and
    /// minified, among entries that are not textured, under a depth test
    /// in some rounds and fogged after the texture in others (issue 998),
    /// are byte for byte the model's. A flat list has no texturing, as it
    /// has no depth, and draws the same entries untextured.
    #[test]
    fn texturing_in_tiles_is_the_models() {
        use crate::model::{render_textured, Textures};
        use crate::op::{DepthMode, Fog, TexMode, LESS};
        use razboj_tile::tex::{encode, level_base, side, texel_offset, Desc};
        use std::collections::HashMap;
        const A: usize = 20;
        const LOGW: usize = 7;
        const W: usize = 1 << LOGW;
        const H: usize = 64;
        const N: usize = 16384;
        const DL: usize = 0xa000;
        const CTRL: usize = 0xfffc;
        // The descriptor, and the levels after it, all clear of the
        // framebuffer and the list.
        const DESC: u32 = 0x8000;
        const BASE: u32 = 0x8040;
        let texel = |l: u32, i: u32, j: u32| {
            ((0x80 + 4 * (i + j) + 16 * l) << 24)
                | ((i * 8 + l * 40) << 16)
                | ((j * 8) << 8)
                | ((i ^ j) * 8 + l * 20)
        };
        let mut x = 0x2545_f491u32;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        let (mut textured, mut flat) = (0, 0);
        for round in 0..25u32 {
            let d = Desc {
                base: BASE,
                log_w: 5,
                log_h: 4,
                levels: 6,
                clamp_s: round % 3 == 1,
                clamp_t: round % 4 == 2,
                min: round % 6,
                mag: (round / 6) % 2,
                class: (round / 5) % 5,
            };
            let mut more: Vec<(usize, u32)> = encode(&d)
                .iter()
                .enumerate()
                .map(|(k, w)| (DESC as usize + 4 * k, *w))
                .collect();
            for l in 0..d.levels {
                for j in 0..side(d.log_h, l) {
                    for i in 0..side(d.log_w, l) {
                        let at = level_base(&d, l) + texel_offset(&d, l, i, j);
                        more.push((at as usize, texel(l, i, j)));
                    }
                }
            }
            // Texel coordinates across a range that magnifies in some
            // rounds and minifies in others.
            let span = [96.0, 384.0, 1536.0][round as usize % 3];
            let mem: HashMap<u32, u32> =
                more.iter().map(|&(a, w)| (a as u32, w)).collect();
            let deep = round % 3 == 2;
            let mut ops = vec![
                Op::Clear {
                    colour: 0x8010_2030,
                },
                Op::Depth(deep.then_some(DepthMode {
                    func: LESS,
                    write: true,
                })),
                Op::Texture(Some(TexMode {
                    desc: DESC,
                    env: round % 5,
                    env_colour: next(),
                })),
            ];
            let fogged = round % 4 == 3;
            if fogged {
                ops.push(Op::Fog(Some(Fog {
                    colour: 0x30_6090 ^ (round << 4),
                    f: [round * 10, 255 - round * 9, 128],
                })));
            }
            for k in 0..4 {
                // Each vertex: where it is, its clip w, and its texel
                // coordinates, some past the texture's edges; then `u q`,
                // `v q` and `q` with the largest `q` one.
                let vs: Vec<((i32, i32), f64, f64, f64)> = (0..3)
                    .map(|_| {
                        let (r, s) = (next(), next());
                        (
                            (
                                (r % (W as u32 * 16)) as i32 - 32,
                                ((r >> 12) % (H as u32 * 16)) as i32 - 32,
                            ),
                            1.0 + (s % 300) as f64 / 100.0,
                            (s >> 9) as f64 % span - span / 3.0,
                            (s >> 17) as f64 % span - span / 3.0,
                        )
                    })
                    .collect();
                let near = vs.iter().map(|v| v.1).fold(f64::MAX, f64::min);
                let uvq = core::array::from_fn(|n| {
                    let (_, w, u, v) = vs[n];
                    let q = near / w;
                    (
                        (u * q * (1u64 << 32) as f64) as i64,
                        (v * q * (1u64 << 32) as f64) as i64,
                        (q * (1u64 << 48) as f64) as u64,
                    )
                });
                let (r, s) = (next(), next());
                ops.push(Op::TexTri {
                    a: vs[0].0,
                    b: vs[1].0,
                    c: vs[2].0,
                    colours: [r | 0x40 << 24, s, r ^ s],
                    shaded: k % 2 == 1,
                    z: [r & 0xffff, s & 0xffff, (r ^ s) >> 16],
                    uvq,
                });
            }
            ops.push(Op::Rect {
                colour: 0xffc0_4020,
                x: (next() % 100) as i32,
                y: (next() % 40) as i32,
                w: 24,
                h: 20,
            });
            let list = assemble(&ops, W, H);
            assert!(list.iter().any(|i| i.tex.to_bool()));
            let tiled = Work {
                more: more.clone(),
                ..Work::tiled(&list, W, H)
            };
            let plain = Work {
                more,
                ..Work::flat(&list)
            };
            let runs = run_works_at::<A, LOGW, H, N, DL, CTRL>(
                &[tiled, plain],
                false,
                false,
            );
            let read = |a: u32| *mem.get(&a).unwrap_or(&0);
            let t = Textures { mem: &read };
            let want = render_textured(&list, W, H, vec![0; W * H], Some(&t));
            let at = want.iter().zip(&runs[0].fb).position(|(p, q)| p != q);
            assert_eq!(
                at,
                None,
                "round {round}: in tiles, {:08x} not {:08x}",
                at.map_or(0, |a| runs[0].fb[a]),
                at.map_or(0, |a| want[a])
            );
            let bare = model::render(&list, W, H);
            textured += (want != bare) as u32;
            if !deep && !fogged {
                assert_eq!(runs[1].fb, bare, "round {round}: flat, untextured");
                flat += 1;
            }
        }
        assert!(textured >= 21, "the texture mattered in {textured} rounds");
        assert!(flat >= 13, "{flat} flat rounds");
    }

    /// What a textured pixel costs (issue 997), and what its refills ask
    /// of the link: one tile of 64 by 64 textured whole, against the same
    /// tile untextured, the difference shared among its pixels. Once with
    /// every pixel on one texel, so that the cache misses once, and once
    /// with a pixel sixteen texels across from the last on a texture 1024
    /// wide, so that a row's 64 blocks share 16 lines and nearly every
    /// pixel misses and refills a line. A refill is one burst of sixteen beats.
    /// And once with three texels a pixel under `LINEAR_MIPMAP_LINEAR`, which
    /// reads eight texels, four from each of two levels.
    #[test]
    fn what_a_textured_pixel_costs() {
        use crate::op::{Op, TexMode};
        use razboj_tile::tex::{
            encode, Desc, LINEAR, LINEAR_MIPMAP_LINEAR, NEAREST,
        };
        const A: usize = 20;
        const LOGW: usize = 6;
        const W: usize = 1 << LOGW;
        const H: usize = 64;
        const N: usize = 16384;
        const DL: usize = 0xa000;
        const CTRL: usize = 0xfffc;
        const DESC: u32 = 0x4000;
        const BASE: u32 = 0x4040;
        let d = Desc {
            base: BASE,
            log_w: 10,
            log_h: 0,
            levels: 1,
            min: NEAREST,
            mag: NEAREST,
            ..Desc::default()
        };
        let words = |d: &Desc| -> Vec<(usize, u32)> {
            encode(d)
                .iter()
                .enumerate()
                .map(|(k, w)| (DESC as usize + 4 * k, *w))
                .collect()
        };
        // A square of two triangles with `u` growing by `step` texels a
        // pixel across, and `q` one throughout.
        let square = |textured: bool, step: i64, d: &Desc| {
            let q = 1u64 << 48;
            let u = |x: i64| (x * step) << 32;
            let s = 64 * 16;
            let mut ops = vec![
                Op::Clear { colour: 0 },
                Op::Texture(textured.then_some(TexMode {
                    desc: DESC,
                    env: 0,
                    env_colour: 0,
                })),
            ];
            for (a, b, c) in
                [((0, 0), (s, 0), (s, s)), ((0, 0), (s, s), (0, s))]
            {
                let at = |p: (i32, i32)| (u(p.0 as i64 / 16), 0, q);
                ops.push(Op::TexTri {
                    a,
                    b,
                    c,
                    colours: [0xff80_8080; 3],
                    shaded: false,
                    z: [0; 3],
                    uvq: [at(a), at(b), at(c)],
                });
            }
            let list = assemble(&ops, W, H);
            Work {
                more: words(d),
                ..Work::tiled(&list, W, H)
            }
        };
        let cost = |step: i64, d: &Desc| {
            let runs = run_works_at::<A, LOGW, H, N, DL, CTRL>(
                &[square(false, step, d), square(true, step, d)],
                false,
                false,
            );
            let px = (W * H) as u64;
            let cycles = runs[1].cycles - 2 * runs[0].cycles;
            let beats = runs[1].reads.1 - 2 * runs[0].reads.1;
            (cycles as f64 / px as f64, beats as f64 / px as f64)
        };
        let (hit, hit_beats) = cost(0, &d);
        let (miss, miss_beats) = cost(16, &d);
        // Three texels a pixel under `LINEAR_MIPMAP_LINEAR`: levels one
        // and two, four texels each, with their refills.
        let tri = Desc {
            levels: 11,
            min: LINEAR_MIPMAP_LINEAR,
            mag: LINEAR,
            ..d
        };
        let (eight, _) = cost(3, &tri);
        println!(
            "a textured pixel: {hit:.1} cycles on a hit, {miss:.1} on a \
             miss, whose refill is {miss_beats:.1} beats ({hit_beats:.3} \
             a pixel on hits); {eight:.1} reading eight texels"
        );
        // A nearest texel of one level: the reciprocal, the level of detail
        // and the texel's own turns (issue 997), about 18.
        assert!(hit < 20.0, "{hit:.1} cycles a hit");
        assert!(miss_beats > 12.0 && miss_beats <= 16.0, "{miss_beats}");
        assert!(miss < hit + 40.0, "{miss:.1} cycles a miss");
        assert!(eight < 90.0, "{eight:.1} cycles for eight texels");
    }

    /// What a load costs (issue 993): the same tile table drawn with its
    /// tiles loaded and with the load bits cleared, the difference in
    /// cycles shared among the tiles loaded. A load reads a burst of the
    /// tile's width a row and writes a pixel a beat, so it costs about
    /// what a write-out does, the reads' latency besides.
    #[test]
    fn a_load_costs_about_a_write_out() {
        use crate::op::{BlendMode, ONE, ONE_MINUS_SRC_ALPHA};
        use razboj_tile::LOAD;
        const A: usize = 20;
        const LOGW: usize = 7;
        const W: usize = 1 << LOGW;
        const H: usize = 64;
        const N: usize = 16384;
        const DL: usize = 0xa000;
        const CTRL: usize = 0xfffc;
        let ops = [
            Op::Blend(Some(BlendMode {
                src: ONE,
                dst: ONE_MINUS_SRC_ALPHA,
            })),
            Op::Rect {
                colour: 0x8040_2010,
                x: 8,
                y: 8,
                w: 112,
                h: 40,
            },
        ];
        let list = assemble(&ops, W, H);
        let loaded = Work::tiled(&list, W, H);
        let tiles = (loaded.count & 0xffff) as usize;
        let mut plain = Work {
            words: loaded.words.clone(),
            count: loaded.count,
            more: Vec::new(),
        };
        for t in 0..tiles {
            plain.words[2 * t + 1] &= !LOAD;
        }
        let with =
            run_works_at::<A, LOGW, H, N, DL, CTRL>(&[loaded], false, false);
        let without =
            run_works_at::<A, LOGW, H, N, DL, CTRL>(&[plain], false, false);
        let each = (with[0].cycles - without[0].cycles) / tiles as u64;
        println!("a load costs {each} cycles a tile, {tiles} tiles");
        let write_out = (razboj_tile::TILE * (razboj_tile::TILE + 1)) as u64;
        assert!(
            each >= write_out && each < 2 * write_out,
            "a load costs {each} cycles a tile, a write-out {write_out}"
        );
    }

    /// Every scene drawn from a tile table is the picture its flat list
    /// draws, byte for byte (issue 1255). There are six scenes of
    /// rectangles, flat and shaded triangles, and clears, many over tile
    /// edges and hanging off the screen, on a screen four tiles across
    /// and one and a half down, so the last row of tiles is cut short.
    /// Each scene is drawn over the last one's picture, so a pixel a
    /// tiled list does not cover has to keep what memory had.
    #[test]
    fn every_scene_is_the_same_in_tiles() {
        const A: usize = 20;
        const LOGW: usize = 8;
        const W: usize = 1 << LOGW;
        const H: usize = 96;
        const N: usize = 32768;
        const DL: usize = 0x18000;
        const CTRL: usize = 0x1fffc;
        let mut x = 0x1234_5678u32;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        let px = |v: u32| (v % (W as u32 + 40)) as i32 - 20;
        let py = |v: u32| (v % (H as u32 + 40)) as i32 - 20;
        let mut scenes = Vec::new();
        for s in 0..6u32 {
            let mut ops = Vec::new();
            if s % 3 == 0 {
                ops.push(Op::Clear {
                    colour: 0x10_2030 + s,
                });
            }
            for _ in 0..6 {
                let (r, q) = (next(), next());
                let colour = q & 0xff_ffff;
                ops.push(match r % 3 {
                    0 => Op::Rect {
                        colour,
                        x: px(r >> 2),
                        y: py(r >> 12),
                        w: ((q >> 8) % 90) as i32 + 1,
                        h: ((q >> 16) % 60) as i32 + 1,
                    },
                    1 => Op::Tri {
                        colour,
                        a: (px(r >> 2), py(r >> 11)),
                        b: (px(q >> 3), py(q >> 13)),
                        c: (px(r ^ q), py((r ^ q) >> 9)),
                    },
                    _ => Op::Gouraud {
                        a: (px(r >> 2) * 16 + 3, py(r >> 11) * 16 + 5),
                        b: (px(q >> 3) * 16 + 9, py(q >> 13) * 16),
                        c: (px(r ^ q) * 16, py((r ^ q) >> 9) * 16 + 11),
                        colours: [q, r, q ^ r].map(|c| c & 0xff_ffff),
                    },
                });
            }
            scenes.push(assemble(&ops, W, H));
        }
        let flat: Vec<Work> = scenes.iter().map(|l| Work::flat(l)).collect();
        let tiled: Vec<Work> =
            scenes.iter().map(|l| Work::tiled(l, W, H)).collect();
        let many = tiled.iter().filter(|t| t.count & 0xffff > 4).count();
        assert!(many >= 3, "scenes over many tiles: {many}");
        let a = run_works_at::<A, LOGW, H, N, DL, CTRL>(&flat, false, false);
        let b = run_works_at::<A, LOGW, H, N, DL, CTRL>(&tiled, false, false);
        for (i, (f, t)) in a.iter().zip(&b).enumerate() {
            let first = f.fb.iter().zip(&t.fb).position(|(p, q)| p != q);
            assert_eq!(first, None, "scene {i}: the first pixel that differs");
            eprintln!(
                "scene {i}: {} tiles, flat {} cycles, tiled {}",
                tiled[i].count & 0xffff,
                f.cycles - if i == 0 { 0 } else { a[i - 1].cycles },
                t.cycles - if i == 0 { 0 } else { b[i - 1].cycles },
            );
        }
    }

    /// A run of a row's pixels goes out as one write burst of at most
    /// sixteen beats (issue 987), on a screen whose rows are four runs
    /// long: a clear is a burst for each sixteen pixels of each row, and
    /// a triangle, whose pixels in a row are one run, a burst for each
    /// sixteen of that run, its beats past the run's end writing nothing.
    /// Each takes one more, for the count's zero, and the pictures are
    /// the model's.
    #[test]
    fn a_run_of_a_rows_pixels_is_one_burst() {
        const LOGW: usize = 6;
        const W: usize = 1 << LOGW;
        const H: usize = 16;
        const N: usize = 4096;
        const DL: usize = 0x1000;
        const CTRL: usize = 0x3800;
        let clear = [bg(0x0012_3456)];
        let got = run::<LOGW, H, N, DL, CTRL>(&clear, false, false);
        assert!(got.fb.iter().all(|&p| p == 0x0012_3456), "the clear");
        assert_eq!(got.bursts, (H * W / 16 + 1) as u64, "clear's bursts");
        assert_eq!(got.writes, (W * H + 1) as u64, "clear's pixels");
        let tri = [Op::Tri {
            colour: 0x00_ff00,
            a: (3, 1),
            b: (61, 7),
            c: (9, 15),
        }];
        let list = assemble(&tri, W, H);
        let got = run::<LOGW, H, N, DL, CTRL>(&tri, false, false);
        assert_eq!(got.fb, model::render(&list, W, H), "the triangle");
        let cover = model::coverage(&list, W, H);
        let runs: Vec<usize> = (0..H)
            .map(|y| {
                cover[y * W..(y + 1) * W].iter().filter(|&&n| n > 0).count()
            })
            .collect();
        let want: usize = runs.iter().map(|n| n.div_ceil(16)).sum();
        assert!(runs.iter().any(|&n| n > 32), "some row is three runs long");
        assert_eq!(got.bursts, (want + 1) as u64, "the triangle's bursts");
        let pixels: usize = runs.iter().sum();
        assert_eq!(got.writes, (pixels + 1) as u64, "the triangle's pixels");
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

    /// Rows past the sixteenth on the board's rows of 1024 pixels
    /// (issue 1178). A pixel's byte offset there is past sixteen bits
    /// from row 16 on, so a rasteriser that forms the offset at sixteen
    /// bits and widens it after wraps every later row into the first
    /// sixteen. The harness's own link is sixteen bits and cannot reach
    /// that far, so this run is on twenty.
    #[test]
    fn rows_past_sixteen_land_where_the_model_puts_them() {
        const LOGW: usize = 10;
        const H: usize = 40;
        let ops = [
            Op::Rect {
                colour: 0x12_3456,
                x: 5,
                y: 14,
                w: 6,
                h: 6,
            },
            Op::Tri {
                colour: 0x65_4321,
                a: (900, 24),
                b: (1000, 38),
                c: (800, 38),
            },
        ];
        let insns = assemble(&ops, 1 << LOGW, H);
        let want = model::render(&insns, 1 << LOGW, H);
        let runs = run_lists_at::<20, LOGW, H, 65536, 0x3_0000, 0x3_8000>(
            &[insns],
            false,
            false,
        );
        let got = &runs[0].fb;
        for y in 0..H {
            for x in 0..1usize << LOGW {
                let i = (y << LOGW) + x;
                assert_eq!(got[i], want[i], "pixel {x},{y}");
            }
        }
        assert!(want.iter().filter(|&&p| p != 0).count() > 1000);
    }
}
