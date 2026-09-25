// SPDX-License-Identifier: Apache-2.0
//! Probe: expected to fail. A unit is `Default`, and a unit holding an
//! array of registers `[Reg<T>; N]` with `N` a const parameter cannot
//! derive it, because the standard library implements `Default` for
//! arrays of each length from 0 to 32 and not for a generic `N`
//! (issue 594). So an array of registers needs a type of its own whose
//! `Default` builds each element, `std::array::from_fn`.
#![allow(dead_code)]

#[derive(Default)]
pub struct Cell(u8);

#[derive(Default)]
pub struct Unit<const N: usize> {
    pub cells: [Cell; N],
}
