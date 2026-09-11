// Probe 1, docs/rust-embedding.md section 6.2 and section 8 item 1.
//
// Does a width that depends on arithmetic over const generic parameters
// compile on stable Rust? This is the question that decides whether the
// numeric library needs nightly, and everything else follows from it.

/// An N-bit unsigned value. The payload is a placeholder; only the type
/// level arithmetic is under test.
pub struct U<const N: usize>(pub u128);

/// Widths stated by hand. This is the fallback if the next one fails.
pub fn mul_32x32(a: U<32>, b: U<32>) -> U<64> {
    U(a.0 * b.0)
}

/// The form the library wants: the result width is the sum of the operand
/// widths. If this needs `generic_const_exprs`, stable is out.
pub fn mul<const A: usize, const B: usize>(a: U<A>, b: U<B>) -> U<{ A + B }> {
    U(a.0 * b.0)
}
