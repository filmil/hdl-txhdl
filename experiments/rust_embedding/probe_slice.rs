// SPDX-License-Identifier: Apache-2.0
// Probe 11. May a bit slice take variable bounds? Against the library's
// `U::slice` and `U::slice_at`. Literal bounds, const-parameter bounds
// and a run-time offset all compile; a run-time width is probe 11b.
use txhdl::types::U;

pub fn opcode(w: U<32>) -> U<8> { w.slice::<0, 8>() }

pub fn field<const LO: usize, const LEN: usize>(w: U<32>) -> U<LEN> { w.slice::<LO, LEN>() }
pub fn use_field(w: U<32>) -> U<4> { field::<12, 4>(w) }

pub fn byte_at(w: U<32>, which: usize) -> U<8> { w.slice_at::<8>(which * 8) }
