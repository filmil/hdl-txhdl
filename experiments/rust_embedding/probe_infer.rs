// SPDX-License-Identifier: Apache-2.0
//! Probe 15. Can a const width be left to inference? A const argument
//! may be `_` on stable Rust since 1.89, so a width the context fixes
//! need not be written: the destination's annotation, a typed use, or
//! the other operand. What the context does not fix stays an error,
//! see `probe_infer_bad`.
use txhdl::types::U;

fn takes8(x: U<8>) -> U<8> {
    x
}

/// The destination is annotated: the slice's width is its width.
pub fn annotated(x: U<32>) -> U<8> {
    let a: U<8> = x.slice::<4, _>();
    a
}

/// The destination is a typed use, two statements later.
pub fn by_use(x: U<32>) -> U<8> {
    let a = x.slice::<4, _>();
    let b = a;
    takes8(b)
}

/// The low operand's width is its type; only the result is stated.
pub fn low_from_operand(x: U<24>, y: U<8>) -> U<32> {
    x.concat::<_, 32>(y)
}

/// The extension's width is the return type's.
pub fn extend_to_return(x: U<8>) -> U<16> {
    x.zext::<_>()
}

/// A compare with a literal fixes nothing, so a slice compared to a
/// number needs its width, and the lowering, which sees no types at
/// all, needs it in every case: see the lowering's rule.
pub fn compared(x: U<32>) -> bool {
    x.slice::<4, 2>() == 2
}
