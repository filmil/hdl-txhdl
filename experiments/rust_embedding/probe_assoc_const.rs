// SPDX-License-Identifier: Apache-2.0
//! Does stable Rust take an address map as an associated constant array
//! of a trait generic over the count, indexed from a const-generic
//! unit? (issue 593)
//!
//! `[usize; N]` as a const parameter needs `adt_const_params`, so a
//! unit over `N` peripherals cannot take its map as one parameter. A
//! trait with `const RANGES: [(usize, usize); N]`, implemented by a
//! marker type, is what stable Rust has instead: the unit is generic
//! over the marker, reads `M::RANGES[i]` in a loop over `0..N`, and the
//! array's length is checked against `N` where the map is written.
//! Expected to compile.

/// A peripheral's base and mask, one per peripheral, first matched
/// first.
pub trait AddrMap<const N: usize> {
    const RANGES: [(usize, usize); N];
}

/// Three ranges of a design's own.
pub struct Three;

impl AddrMap<3> for Three {
    const RANGES: [(usize, usize); 3] =
        [(0x1000, 0xf000), (0x2000, 0xf000), (0x3000, 0xff00)];
}

/// A decoder over `N` ranges, generic over the map that names them.
pub struct Decode<const N: usize, M: AddrMap<N>>(core::marker::PhantomData<M>);

impl<const N: usize, M: AddrMap<N>> Decode<N, M> {
    /// Which ranges hold `a`.
    pub fn hits(a: usize) -> [bool; N] {
        let mut h = [false; N];
        for (i, hit) in h.iter_mut().enumerate() {
            *hit = (a & M::RANGES[i].1) == M::RANGES[i].0;
        }
        h
    }
}

/// The second range holds 0x2040 and no other does.
pub fn check() -> bool {
    Decode::<3, Three>::hits(0x2040) == [false, true, false]
}
