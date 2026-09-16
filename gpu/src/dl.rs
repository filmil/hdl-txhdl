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
//!   word 3  [11:0] ax       [27:16] ay
//!   word 4  [11:0] bx       [27:16] by
//!   word 5  [11:0] cx       [27:16] cy
//! ```
//!
//! Beside the list is one more word, the count: how many instructions
//! the list holds. The rasteriser reads it until it is not zero, and
//! that is how a program says the list is ready. A program therefore
//! writes the list first and the count last.
//!
//! The count sits beside the list and not in it, so whoever lays the
//! two out leaves the list room for the longest it will hold: a list
//! of `n` instructions reaches `base + (n << BYTE_SHIFT)`, and the
//! count has to be at or past that.
use txhdl::types::U;

use crate::op::{Insn, Kind};

// begin{format}
/// Words an instruction takes, a power of two.
pub const WORDS: usize = 8;
/// The shift from an instruction's index to its byte address.
pub const BYTE_SHIFT: usize = 5;
/// Words of an instruction that carry anything.
pub const USED: usize = 6;

/// One instruction as the words a program writes.
pub fn encode(i: &Insn) -> [u32; WORDS] {
    let k = match i.kind {
        Kind::Clear => 0u32,
        Kind::Rect => 1,
        Kind::Tri => 2,
    };
    let lo = |v: u128| v as u32;
    let mut w = [0u32; WORDS];
    w[0] = k | (lo(i.colour.raw()) << 2);
    w[1] = lo(i.x0.raw()) | (lo(i.y0.raw()) << 16);
    w[2] = lo(i.x1.raw()) | (lo(i.y1.raw()) << 16);
    w[3] = lo(i.ax.raw()) | (lo(i.ay.raw()) << 16);
    w[4] = lo(i.bx.raw()) | (lo(i.by.raw()) << 16);
    w[5] = lo(i.cx.raw()) | (lo(i.cy.raw()) << 16);
    w
}
// end{format}

/// The words of an instruction read back, for a model or a test.
pub fn decode(w: &[u32]) -> Insn {
    let kind = match w[0] & 3 {
        0 => Kind::Clear,
        1 => Kind::Rect,
        _ => Kind::Tri,
    };
    let f = |word: usize, shift: usize, bits: u32| {
        (w[word] >> shift) & ((1 << bits) - 1)
    };
    Insn {
        kind,
        colour: U::from((w[0] >> 2) & 0xff_ffff),
        x0: U::from(f(1, 0, 10)),
        y0: U::from(f(1, 16, 10)),
        x1: U::from(f(2, 0, 10)),
        y1: U::from(f(2, 16, 10)),
        ax: U::from(f(3, 0, 12)),
        ay: U::from(f(3, 16, 12)),
        bx: U::from(f(4, 0, 12)),
        by: U::from(f(4, 16, 12)),
        cx: U::from(f(5, 0, 12)),
        cy: U::from(f(5, 16, 12)),
    }
}

/// A whole display list as the words a program writes, the
/// instructions one after another at [`WORDS`] words each.
pub fn image(list: &[Insn]) -> Vec<u32> {
    let mut out = vec![0u32; list.len() * WORDS];
    for (i, ins) in list.iter().enumerate() {
        out[i * WORDS..i * WORDS + WORDS].copy_from_slice(&encode(ins));
    }
    out
}

/// What goes through the format and comes back unchanged.
#[cfg(test)]
mod tests {
    use super::{decode, encode, image, WORDS};
    use crate::op::{assemble, Op};

    #[test]
    fn an_instruction_survives_the_format() {
        let ops = vec![
            Op::Clear { colour: 0x12_3456 },
            Op::Rect {
                colour: 0x65_4321,
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
            assert_eq!(got.x0.raw(), want.x0.raw(), "x0 of {i}");
            assert_eq!(got.y1.raw(), want.y1.raw(), "y1 of {i}");
            assert_eq!(got.ax.raw(), want.ax.raw(), "ax of {i}");
            assert_eq!(got.cy.raw(), want.cy.raw(), "cy of {i}");
        }
        // A vertex off the screen is two's complement in twelve bits
        // and comes back as it went in.
        let t = decode(&words[2 * WORDS..]);
        assert_eq!(crate::op::signed(t.ax), -4, "a negative vertex");
    }
}
