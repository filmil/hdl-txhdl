// SPDX-License-Identifier: Apache-2.0
//! The display list in memory: the words a program writes and the
//! rasteriser reads.
//!
//! This is the one format that software and hardware both have to
//! agree on, so it is stated once, here, and the encoder below and
//! the rasteriser's fetch are both written against it.
//!
//! An instruction is [`WORDS`] words, of which [`USED`] carry
//! anything, and no field crosses a word: a program writes a field
//! with a shift and a mask, and the rasteriser reads it with a slice
//! at a constant offset. The count is a power of two so that the
//! address of the `i`th instruction is `base + (i << BYTE_SHIFT)`,
//! which is a shift and not a multiply.
//!
//! ```text
//!   word 0   [1:0] kind      [25:2] colour
//!   word 1   [9:0] x0       [25:16] y0
//!   word 2   [9:0] x1       [25:16] y1
//!   word 3  [15:0] ax       [31:16] ay
//!   word 4  [15:0] bx       [31:16] by
//!   word 5  [15:0] cx       [31:16] cy
//!   word 6  r0      word 7  rdx     word 8  rdy
//!   word 9  g0      word 10 gdx     word 11 gdy
//!   word 12 b0      word 13 bdx     word 14 bdy
//!   word 15  [7:0] alpha  [8] depth  [11:9] the comparison  [12] write
//! ```
//!
//! Words 6 to 14 are a shaded triangle's three planes, each the
//! channel's value at the box's first pixel and its two steps, with
//! sixteen bits of fraction; the other kinds leave them zero. Word 15
//! is every entry's alpha, which the pixel takes in its top byte.
//!
//! An entry with the depth bit set tests depth (issue 992) and takes a
//! second slot of [`WORDS`] words, right after it: its depth plane in
//! the first three, the value at the box's first pixel and the steps a
//! pixel right and a row down, with `op::ZFRAC` bits of fraction. The
//! count says entries, not slots. Depth lives only in Razboj's tile
//! buffer, so only a tiled list tests it; in a flat list such an entry
//! draws as if depth were off, and a program that wants depth rings a
//! tile table.
//!
//! Beside the list is one more word, the count: how many instructions
//! the list holds, in its low sixteen bits. The rasteriser reads it
//! until it is not zero, and that is how a program says the list is
//! ready. A program therefore writes the list first and the count last.
//!
//! The count sits beside the list and not in it, so whoever lays the
//! two out leaves the list room for the longest it will hold: a list
//! of `n` instructions reaches `base + (n << BYTE_SHIFT)`, and the
//! count has to be at or past that.
use txhdl::types::{Bit, U};

use crate::op::{Insn, Kind};

// begin{format}
/// Words an instruction takes, a power of two.
pub const WORDS: usize = 16;
/// The shift from an instruction's index to its byte address.
pub const BYTE_SHIFT: usize = 6;
/// Words of an instruction that carry anything.
pub const USED: usize = 16;

/// One instruction as the words a program writes.
pub fn encode(i: &Insn) -> [u32; WORDS] {
    let k = match i.kind {
        Kind::Clear => 0u32,
        Kind::Rect => 1,
        Kind::Tri => 2,
        Kind::Shaded => 3,
    };
    let lo = |v: u128| v as u32;
    let mut w = [0u32; WORDS];
    w[0] = k | (lo(i.colour.raw()) << 2);
    w[1] = lo(i.x0.raw()) | (lo(i.y0.raw()) << 16);
    w[2] = lo(i.x1.raw()) | (lo(i.y1.raw()) << 16);
    w[3] = lo(i.ax.raw()) | (lo(i.ay.raw()) << 16);
    w[4] = lo(i.bx.raw()) | (lo(i.by.raw()) << 16);
    w[5] = lo(i.cx.raw()) | (lo(i.cy.raw()) << 16);
    let planes = [i.r0, i.rdx, i.rdy, i.g0, i.gdx, i.gdy, i.b0, i.bdx, i.bdy];
    for (k, p) in planes.iter().enumerate() {
        w[6 + k] = lo(p.raw());
    }
    w[15] = lo(i.alpha.raw())
        | ((i.depth.to_bool() as u32) << 8)
        | (lo(i.zfunc.raw()) << 9)
        | ((i.zwrite.to_bool() as u32) << 12);
    w
}

/// The second slot of an entry that tests depth (issue 992): its depth
/// plane, the value at the box's first pixel and the two steps, in its
/// first three words. `None` for an entry that does not, which takes
/// one slot.
pub fn encode_ext(i: &Insn) -> Option<[u32; WORDS]> {
    if !i.depth.to_bool() {
        return None;
    }
    let mut w = [0u32; WORDS];
    w[0] = i.z0.raw() as u32;
    w[1] = i.zdx.raw() as u32;
    w[2] = i.zdy.raw() as u32;
    Some(w)
}
// end{format}

/// The words of an instruction read back, for a model or a test.
pub fn decode(w: &[u32]) -> Insn {
    let kind = match w[0] & 3 {
        0 => Kind::Clear,
        1 => Kind::Rect,
        2 => Kind::Tri,
        _ => Kind::Shaded,
    };
    let f = |word: usize, shift: usize, bits: u32| {
        (w[word] >> shift) & ((1 << bits) - 1)
    };
    Insn {
        kind,
        colour: U::from((w[0] >> 2) & 0xff_ffff),
        alpha: U::from(w[15] & 0xff),
        x0: U::from(f(1, 0, 10)),
        y0: U::from(f(1, 16, 10)),
        x1: U::from(f(2, 0, 10)),
        y1: U::from(f(2, 16, 10)),
        ax: U::from(f(3, 0, 16)),
        ay: U::from(f(3, 16, 16)),
        bx: U::from(f(4, 0, 16)),
        by: U::from(f(4, 16, 16)),
        cx: U::from(f(5, 0, 16)),
        cy: U::from(f(5, 16, 16)),
        r0: U::from(w[6]),
        rdx: U::from(w[7]),
        rdy: U::from(w[8]),
        g0: U::from(w[9]),
        gdx: U::from(w[10]),
        gdy: U::from(w[11]),
        b0: U::from(w[12]),
        bdx: U::from(w[13]),
        bdy: U::from(w[14]),
        depth: Bit::from((w[15] >> 8) & 1 == 1),
        zfunc: U::from((w[15] >> 9) & 7),
        zwrite: Bit::from((w[15] >> 12) & 1 == 1),
        ..Insn::default()
    }
}

/// A list's words read back as its instructions, an entry that tests
/// depth taking its second slot with it.
pub fn decode_list(words: &[[u32; WORDS]]) -> Vec<Insn> {
    let mut out = Vec::new();
    let mut k = 0;
    while k < words.len() {
        let mut i = decode(&words[k]);
        if i.depth.to_bool() {
            let e = &words[k + 1];
            (i.z0, i.zdx, i.zdy) =
                (U::from(e[0]), U::from(e[1]), U::from(e[2]));
            k += 1;
        }
        out.push(i);
        k += 1;
    }
    out
}

/// A whole display list as the words a program writes, the
/// instructions one after another at [`WORDS`] words each.
pub fn image(list: &[Insn]) -> Vec<u32> {
    let mut out = Vec::with_capacity(list.len() * WORDS);
    for ins in list {
        out.extend_from_slice(&encode(ins));
        if let Some(e) = encode_ext(ins) {
            out.extend_from_slice(&e);
        }
    }
    out
}

/// What goes through the format and comes back unchanged.
#[cfg(test)]
mod tests {
    use super::{decode, image, WORDS};
    use crate::op::{assemble, Op};

    #[test]
    fn an_instruction_survives_the_format() {
        let ops = vec![
            Op::Clear { colour: 0x12_3456 },
            Op::Rect {
                colour: 0x8065_4321,
                x: 3,
                y: 5,
                w: 7,
                h: 9,
            },
            Op::Tri {
                colour: 0xab_cdef,
                a: (-4, 2),
                b: (30, 6),
                c: (9, 28),
            },
        ];
        let list = assemble(&ops, 32, 32);
        assert_eq!(list.len(), 3, "every entry drew something");
        let words = image(&list);
        assert_eq!(words.len(), 3 * WORDS);
        for (i, want) in list.iter().enumerate() {
            let got = decode(&words[i * WORDS..]);
            assert_eq!(got.kind, want.kind, "kind of {i}");
            assert_eq!(got.colour.raw(), want.colour.raw(), "colour of {i}");
            assert_eq!(got.alpha.raw(), want.alpha.raw(), "alpha of {i}");
            assert_eq!(got.x0.raw(), want.x0.raw(), "x0 of {i}");
            assert_eq!(got.y1.raw(), want.y1.raw(), "y1 of {i}");
            assert_eq!(got.ax.raw(), want.ax.raw(), "ax of {i}");
            assert_eq!(got.cy.raw(), want.cy.raw(), "cy of {i}");
        }
        // A vertex off the screen is two's complement in sixteen bits,
        // in sixteenths of a pixel, and comes back as it went in.
        let t = decode(&words[2 * WORDS..]);
        assert_eq!(crate::op::signed(t.ax), -4 * 16, "a negative vertex");
        // The alpha is word 15's low byte, apart from the colour.
        let r = decode(&words[WORDS..]);
        assert_eq!((r.alpha.raw(), r.colour.raw()), (0x80, 0x65_4321));
    }
}
