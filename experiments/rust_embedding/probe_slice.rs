// Probe 11. Can a bit slice take variable parameters?
//
// Three cases, and they do not have the same answer:
//   a. literal bounds
//   b. bounds that are const generic parameters, not literals
//   c. a bound that is a run-time value

pub struct U<const N: usize>(pub u128);

impl<const N: usize> U<N> {
    /// Static offset and static width. Both are const generic
    /// parameters, so both are known when the design is elaborated.
    pub fn slice<const LO: usize, const LEN: usize>(&self) -> U<LEN> {
        U((self.0 >> LO) & ((1u128 << LEN) - 1))
    }

    /// A run-time offset with a static width. The width is the type, so
    /// it cannot vary; the offset is just a number, so it can. In
    /// hardware this is a barrel shifter feeding a fixed-width slice.
    pub fn slice_at<const LEN: usize>(&self, lo: usize) -> U<LEN> {
        U((self.0 >> lo) & ((1u128 << LEN) - 1))
    }
}

// (a) literals.
pub fn opcode(w: U<32>) -> U<8> {
    w.slice::<0, 8>()
}

// (b) the bounds are generic parameters rather than literals, so a
// generic function can slice without knowing the numbers.
pub fn field<const LO: usize, const LEN: usize>(w: U<32>) -> U<LEN> {
    w.slice::<LO, LEN>()
}

pub fn use_field(w: U<32>) -> U<4> {
    field::<12, 4>(w)
}

// (c) a run-time offset, static width.
pub fn byte_at(w: U<32>, which: usize) -> U<8> {
    w.slice_at::<8>(which * 8)
}
