// SPDX-License-Identifier: Apache-2.0
//! A display list in tiles, on the host: `razboj_tile`'s binning of the
//! instructions the assembler writes, and the check that a binned list
//! draws exactly what the list does (issue 1157).
//!
//! The binning itself is in `razboj_tile`, which has no standard
//! library so that a Vreteno program can use it. This module only
//! gives it room and reads the instructions back, so the tests here
//! check the same code a program runs.
use crate::dl::{decode, encode, WORDS};
use crate::op::Insn;
use razboj_tile::{bin, Binned, ENTRIES_AT, MAX_TILES, TILED, TILE_WORDS};

/// A list binned into tiles: the tile table's records, and the entries,
/// each tile's in the list's order, one tile after another.
pub struct Tiled {
    pub tiles: Vec<[u32; TILE_WORDS]>,
    pub entries: Vec<Insn>,
}

/// `list`, on a screen of `sw` by `sh` pixels, binned into tiles.
pub fn tiled(list: &[Insn], sw: usize, sh: usize) -> Tiled {
    let words: Vec<[u32; WORDS]> = list.iter().map(encode).collect();
    // The most an entry can take is a record in every tile.
    let mut entries = vec![[0u32; WORDS]; list.len() * MAX_TILES];
    let mut tiles = vec![[0u32; TILE_WORDS]; MAX_TILES];
    let Binned {
        tiles: t,
        entries: n,
    } = bin(&words, sw as u32, sh as u32, &mut entries, &mut tiles)
        .expect("room for every entry in every tile");
    Tiled {
        tiles: tiles[..t].to_vec(),
        entries: entries[..n].iter().map(|w| decode(w)).collect(),
    }
}

/// `list` binned into tiles as a program lays it out for the rasteriser
/// (issue 1255): the words that go at the list's address, the tile
/// table and then, from `ENTRIES_AT` past it, the entries; and the count
/// word, the number of tiles with the bit that says they are tiles.
pub fn image(list: &[Insn], sw: usize, sh: usize) -> (Vec<u32>, u32) {
    let t = tiled(list, sw, sh);
    let mut words = vec![0u32; ENTRIES_AT / 4 + t.entries.len() * WORDS];
    for (i, rec) in t.tiles.iter().enumerate() {
        words[i * TILE_WORDS..(i + 1) * TILE_WORDS].copy_from_slice(rec);
    }
    for (i, e) in t.entries.iter().enumerate() {
        let at = ENTRIES_AT / 4 + i * WORDS;
        words[at..at + WORDS].copy_from_slice(&encode(e));
    }
    (words, t.tiles.len() as u32 | TILED)
}

#[cfg(test)]
mod tests {
    use super::tiled;
    use crate::model::{coverage, render};
    use crate::op::{assemble, Insn, Kind, Op};
    use razboj_tile::{tile, TILE};

    const W: usize = 640;
    const H: usize = 480;

    /// The words of the instruction format and the binning agree on how
    /// many words an instruction is.
    #[test]
    fn the_two_crates_agree_on_an_instructions_words() {
        assert_eq!(crate::dl::WORDS, razboj_tile::WORDS);
    }

    /// The box an entry walks, both ends included.
    fn box_of(i: &Insn) -> (u32, u32, u32, u32) {
        let r = |v: txhdl::types::U<10>| v.raw() as u32;
        (r(i.x0), r(i.y0), r(i.x1), r(i.y1))
    }

    /// `ops` drawn untiled and binned, pixel for pixel, and every
    /// binned entry inside its tile. Returns the binned list.
    fn agree(ops: &[Op], what: &str) -> Vec<Insn> {
        let list = assemble(ops, W, H);
        let t = tiled(&list, W, H);
        for rec in &t.tiles {
            let (first, count) = (rec[0] & 0xffff, rec[0] >> 16);
            let (x, y) = (rec[1] & 0x3ff, (rec[1] >> 16) & 0x3ff);
            assert_eq!((x % TILE, y % TILE), (0, 0), "{what}: a tile's origin");
            for e in &t.entries[first as usize..(first + count) as usize] {
                let (x0, y0, x1, y1) = box_of(e);
                assert!(
                    x0 >= x && y0 >= y && x1 < x + TILE && y1 < y + TILE,
                    "{what}: an entry outside its tile at {x}, {y}"
                );
            }
        }
        assert_eq!(
            render(&t.entries, W, H),
            render(&list, W, H),
            "{what}: the binned picture"
        );
        t.entries
    }

    /// An entry moved by `dx`, `dy` pixels.
    fn moved(op: Op, dx: i32, dy: i32) -> Op {
        let p = |(x, y): (i32, i32)| (x + dx, y + dy);
        let q = |(x, y): (i32, i32)| (x + 16 * dx, y + 16 * dy);
        match op {
            Op::Rect { colour, x, y, w, h } => Op::Rect {
                colour,
                x: x + dx,
                y: y + dy,
                w,
                h,
            },
            Op::Tri { colour, a, b, c } => Op::Tri {
                colour,
                a: p(a),
                b: p(b),
                c: p(c),
            },
            Op::TriQ4 { colour, a, b, c } => Op::TriQ4 {
                colour,
                a: q(a),
                b: q(b),
                c: q(c),
            },
            Op::Gouraud { a, b, c, colours } => Op::Gouraud {
                a: q(a),
                b: q(b),
                c: q(c),
                colours,
            },
            Op::Scissor { x, y, w, h } => Op::Scissor {
                x: x + dx,
                y: y + dy,
                w,
                h,
            },
            clear => clear,
        }
    }

    /// The demonstration's house, four times over at places where each
    /// lies across a tile's edge or a tile's corner.
    #[test]
    fn the_house_across_tile_edges() {
        let mut ops = vec![];
        for (dx, dy) in [(0, 0), (30, 20), (100, 40), (560, 430)] {
            ops.extend(
                crate::scene::house().into_iter().map(|o| moved(o, dx, dy)),
            );
        }
        agree(&ops, "the houses");
    }

    /// Shaded triangles over many tiles, one hanging off the screen and
    /// one under a flat triangle, each pixel's colour the untiled one to
    /// the bit (issue 989).
    #[test]
    fn shaded_triangles_across_tile_edges() {
        let q = |x: i32, y: i32| (x * 16 + 5, y * 16 + 11);
        let ops = vec![
            Op::Clear { colour: 0x10_1010 },
            Op::Gouraud {
                a: q(10, 15),
                b: q(600, 90),
                c: q(200, 470),
                colours: [0xff_0000, 0x00_ff00, 0x00_00ff],
            },
            Op::Gouraud {
                a: q(-80, 300),
                b: q(700, 250),
                c: q(330, 600),
                colours: [0x80_ff_ff_00, 0x00_ff_ff, 0xff_00ff],
            },
            Op::Tri {
                colour: 0x40_4040,
                a: (120, 100),
                b: (300, 130),
                c: (180, 260),
            },
        ];
        let binned = agree(&ops, "the shaded triangles");
        let shaded = binned.iter().filter(|i| i.kind == Kind::Shaded).count();
        assert!(shaded > 40, "the two shaded triangles over many tiles");
    }

    /// Why the planes are stepped rather than worked out again: a shaded
    /// triangle encoded afresh inside each tile it touches rounds each
    /// tile's start at a different place, and draws a different picture
    /// from the triangle drawn whole, while the stepped planes draw the
    /// same one (`docs/razboj-tiles.md`, section 4).
    #[test]
    fn planes_worked_out_per_tile_would_differ() {
        let q = |x: i32, y: i32| (x * 16 + 5, y * 16 + 11);
        let mut differ = 0;
        for k in 0..20 {
            let op = Op::Gouraud {
                a: q(10 + k, 15),
                b: q(600, 90 + 3 * k),
                c: q(200 - 2 * k, 470),
                colours: [0xff_0000 + k as u32, 0xff00, 0xff + 0x10 * k as u32],
            };
            let whole = op.encode(W, H).expect("a triangle on the screen");
            let t = tiled(&[whole], W, H);
            assert_eq!(render(&t.entries, W, H), render(&[whole], W, H));
            let afresh: Vec<Insn> = t
                .tiles
                .iter()
                .filter_map(|r| {
                    let (x, y) = (r[1] & 0x3ff, (r[1] >> 16) & 0x3ff);
                    op.encode_in(
                        tile(x / TILE, y / TILE, W as u32, H as u32),
                        W,
                        H,
                    )
                })
                .collect();
            if render(&afresh, W, H) != render(&[whole], W, H) {
                differ += 1;
            }
        }
        assert!(differ > 0, "worked out afresh, every tile agreed");
    }

    /// The mesh of issue 988 over the whole screen, its cells a tile
    /// each, so that every tile edge is a mesh edge and every corner a
    /// vertex moved by sixteenths: the binned mesh still covers every
    /// pixel exactly once.
    #[test]
    fn a_mesh_over_the_tiles_draws_every_pixel_once() {
        let (cols, rows) = (W as i32 / 64, H as i32 / 64 + 1);
        let jitter = |i: i32, j: i32| -> (i32, i32) {
            if i == 0 || j == 0 || i == cols || j == rows {
                (0, 0)
            } else {
                ((i * 7 + j * 3) % 11 - 5, (i * 5 + j * 7) % 13 - 6)
            }
        };
        let v = |i: i32, j: i32| {
            let (dx, dy) = jitter(i, j);
            (1024 * i + dx, 1024 * j + dy)
        };
        let mut ops = vec![Op::Clear { colour: 0 }];
        let mut colour = 0x10_0000;
        for j in 0..rows {
            for i in 0..cols {
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
        let binned = agree(&ops, "the mesh");
        let mesh: Vec<Insn> =
            binned.into_iter().filter(|i| i.colour.raw() != 0).collect();
        let cover = coverage(&mesh, W, H);
        assert!(cover.iter().all(|&n| n == 1), "every pixel covered once");
    }

    /// Under a scissor box that crosses tiles, a clear and a triangle
    /// stay inside the box, binned or not (issue 990).
    #[test]
    fn a_scissor_box_across_tiles() {
        let ops = vec![
            Op::Clear { colour: 0x20_2020 },
            Op::Scissor {
                x: 50,
                y: 40,
                w: 300,
                h: 150,
            },
            Op::Clear { colour: 0x00_8000 },
            Op::Tri {
                colour: 0xc0_0000,
                a: (0, 0),
                b: (500, 60),
                c: (90, 400),
            },
        ];
        agree(&ops, "the scissor box");
    }

    /// Scenes of pseudorandom rectangles and triangles, flat and shaded,
    /// from a literal seed, many of them hanging off the screen.
    #[test]
    fn pseudorandom_scenes_binned() {
        let mut x = 0x2545_f491u32;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        for scene in 0..8 {
            let mut ops = vec![Op::Clear { colour: 0x30_3030 }];
            for _ in 0..12 {
                let r = next();
                let s = next();
                let p = |v: u32, span: u32| (v % span) as i32 - 60;
                let colour = s;
                ops.push(match r & 3 {
                    0 => Op::Rect {
                        colour,
                        x: p(r >> 2, 720),
                        y: p(s >> 3, 560),
                        w: (r >> 20) as i32 % 200 + 1,
                        h: (s >> 20) as i32 % 150 + 1,
                    },
                    1 => Op::Tri {
                        colour,
                        a: (p(r >> 2, 760), p(s >> 2, 600)),
                        b: (p(r >> 7, 760), p(s >> 7, 600)),
                        c: (p(r >> 12, 760), p(s >> 12, 600)),
                    },
                    _ => {
                        let t = next();
                        let q = |v: u32| p(v, 760) * 16 + (v % 16) as i32;
                        Op::Gouraud {
                            a: (q(r >> 2), q(s >> 2)),
                            b: (q(r >> 9), q(s >> 9)),
                            c: (q(t >> 2), q(t >> 9)),
                            colours: [r, s, t].map(|c| c & 0xff_ffff),
                        }
                    }
                });
            }
            agree(&ops, &format!("scene {scene}"));
        }
    }
}
