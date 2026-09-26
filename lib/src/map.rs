// SPDX-License-Identifier: Apache-2.0
//! An address map as a trait constant (issue 593).
//!
//! A unit over `N` peripherals decodes an address against `N` ranges,
//! and stable Rust has no array const parameter to carry them: an
//! `[usize; N]` as a const parameter needs `adt_const_params`. So a
//! map is a type, and the ranges are an associated constant of it: a
//! design writes a marker type, implements [`AddrMap`] for it with the
//! count, and names the marker where it names the unit,
//! `Decode<3, MyMap>`. The unit reads `M::RANGES[i]` in a loop over
//! `0..N`; the lowering unrolls the loop when `lowered` runs and works
//! each base and mask out as a number of the netlist, as it does a
//! const parameter, so the netlist of `Decode<3, MyMap>` decodes the
//! three ranges `MyMap` states, and the array's length is checked
//! against `N` where the map is written.
//!
//! This is the form the routers and the bridges of `txhdl_parts` move
//! onto, in place of a `BASE` and a `MASK` const parameter per
//! peripheral (issue 500).

/// The ranges of `N` peripherals: each a base and a mask, an address
/// belonging to the peripheral when `addr & mask == base`. The first
/// range that matches is the one, so ranges may nest if the finer is
/// first.
pub trait AddrMap<const N: usize> {
    /// The base and the mask of each peripheral, in the order the
    /// unit's ports are in.
    const RANGES: [(usize, usize); N];
    /// What each range is, in the same order: a few words a table of
    /// the map prints beside its base, so that a document lists the map
    /// the design decodes rather than a copy of it (issue 444). A map
    /// that names nothing leaves each one empty.
    const NAMES: [&'static str; N] = [""; N];
}
