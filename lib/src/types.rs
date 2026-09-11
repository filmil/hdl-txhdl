// SPDX-License-Identifier: Apache-2.0
//! Value types, paralleling the primitives rather than reusing them.
//!
//! Rust's integers are the wrong types for hardware, and the reason is
//! not width. `u32` has no way to say that nobody has driven this wire
//! yet. So: [`Bit`] is two valued and what synthesis maps; [`Logic`] is
//! nine valued, IEEE 1164, and what simulation needs; [`U`] and [`I`]
//! are the numeric vectors; [`logic::Vec`] is a vector of [`Logic`] for
//! when unknowns must propagate.

/// Two valued. `Default` is `Zero`, because a synthesised register
/// leaves reset at a defined value.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Bit {
    #[default]
    Zero,
    One,
}

impl Bit {
    pub fn from_bool(b: bool) -> Self {
        if b {
            Bit::One
        } else {
            Bit::Zero
        }
    }
    pub fn to_bool(self) -> bool {
        matches!(self, Bit::One)
    }
    pub fn not(self) -> Self {
        Self::from_bool(!self.to_bool())
    }
    pub fn and(self, o: Bit) -> Self {
        Self::from_bool(self.to_bool() && o.to_bool())
    }
    pub fn or(self, o: Bit) -> Self {
        Self::from_bool(self.to_bool() || o.to_bool())
    }
}

/// Nine valued, in IEEE 1164 order. `Default` is `U`: a signal nobody
/// has driven is uninitialised, not zero. That difference is the reason
/// the type exists.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Logic {
    #[default]
    U,
    X,
    Zero,
    One,
    Z,
    W,
    L,
    H,
    DontCare,
}

impl Logic {
    const fn index(self) -> usize {
        match self {
            Logic::U => 0,
            Logic::X => 1,
            Logic::Zero => 2,
            Logic::One => 3,
            Logic::Z => 4,
            Logic::W => 5,
            Logic::L => 6,
            Logic::H => 7,
            Logic::DontCare => 8,
        }
    }

    const fn from_index(i: usize) -> Self {
        match i {
            0 => Logic::U,
            1 => Logic::X,
            2 => Logic::Zero,
            3 => Logic::One,
            4 => Logic::Z,
            5 => Logic::W,
            6 => Logic::L,
            7 => Logic::H,
            _ => Logic::DontCare,
        }
    }

    /// The IEEE 1164 resolution table, for two drivers on one wire.
    pub fn resolve(self, other: Logic) -> Logic {
        const T: [[usize; 9]; 9] = [
            [0, 1, 1, 1, 1, 1, 1, 1, 1],
            [1, 1, 1, 1, 1, 1, 1, 1, 1],
            [1, 1, 2, 1, 2, 2, 2, 2, 1],
            [1, 1, 1, 3, 3, 3, 3, 3, 1],
            [1, 1, 2, 3, 4, 5, 6, 7, 1],
            [1, 1, 2, 3, 5, 5, 5, 5, 1],
            [1, 1, 2, 3, 6, 5, 6, 5, 1],
            [1, 1, 2, 3, 7, 5, 5, 7, 1],
            [1, 1, 1, 1, 1, 1, 1, 1, 1],
        ];
        Logic::from_index(T[self.index()][other.index()])
    }

    /// Narrowing to two values. `None` where the value is not a definite
    /// 0 or 1, which is where a design would otherwise synthesise
    /// something it had not stated.
    pub fn to_bit(self) -> Option<Bit> {
        match self {
            Logic::Zero | Logic::L => Some(Bit::Zero),
            Logic::One | Logic::H => Some(Bit::One),
            _ => None,
        }
    }

    pub fn from_bit(b: Bit) -> Self {
        match b {
            Bit::Zero => Logic::Zero,
            Bit::One => Logic::One,
        }
    }

    pub fn is_defined(self) -> bool {
        self.to_bit().is_some()
    }
}

/// An N-bit unsigned value.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct U<const N: usize>(u128);

impl<const N: usize> U<N> {
    pub const WIDTH: usize = N;
    const MASK: u128 = if N >= 128 {
        u128::MAX
    } else {
        (1u128 << N) - 1
    };

    pub const fn new(v: u128) -> Self {
        U(v & Self::MASK)
    }
    pub const fn raw(self) -> u128 {
        self.0
    }
    pub fn bit(self, i: usize) -> Bit {
        Bit::from_bool((self.0 >> i) & 1 == 1)
    }

    /// Same-width, wrapping. The result width is a standalone parameter,
    /// so this is stable Rust. The operand is anything that converts, so
    /// a literal is written as a literal.
    pub fn wrapping_add(self, o: impl Into<Self>) -> Self {
        Self::new(self.0.wrapping_add(o.into().0))
    }
    pub fn wrapping_sub(self, o: impl Into<Self>) -> Self {
        Self::new(self.0.wrapping_sub(o.into().0))
    }

    /// A multiply with the result width stated: `a.mul::<64>(b)`. The
    /// width is a standalone parameter, so this is stable; `U<{A + B}>`
    /// would not be.
    pub fn mul<const M: usize>(self, o: impl Into<Self>) -> U<M> {
        U::<M>::new(self.0 * o.into().0)
    }

    /// Resize to a stated width. `M` is standalone, so stable.
    pub fn resize<const M: usize>(self) -> U<M> {
        U::<M>::new(self.0)
    }

    /// A slice. Offset and width are const parameters; an offset may
    /// also be a run-time value through [`U::slice_at`], but a width may
    /// not, because the width is the type.
    pub fn slice<const LO: usize, const LEN: usize>(self) -> U<LEN> {
        U::<LEN>::new(self.0 >> LO)
    }

    pub fn slice_at<const LEN: usize>(self, lo: usize) -> U<LEN> {
        U::<LEN>::new(self.0 >> lo)
    }
}

// Conversions from the primitive integers, so a value is written as a
// number and the width comes from the context. `i32` is included because
// an integer literal with no other constraint is an `i32`, and a literal
// is the common case; a negative one is a bug, and is caught in debug.
macro_rules! from_int {
    ($($t:ty),*) => { $(
        impl<const N: usize> From<$t> for U<N> {
            fn from(v: $t) -> Self { Self::new(v as u128) }
        }
    )* };
}
from_int!(u8, u16, u32, u64, u128, usize);

impl<const N: usize> From<i32> for U<N> {
    fn from(v: i32) -> Self {
        debug_assert!(v >= 0, "a negative literal into an unsigned U<{N}>");
        Self::new(v as u128)
    }
}

/// An N-bit signed value, two's complement.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct I<const N: usize>(i128);

impl<const N: usize> I<N> {
    pub const WIDTH: usize = N;

    pub fn new(v: i128) -> Self {
        let shift = 128 - N;
        I((v << shift) >> shift)
    }
    pub const fn raw(self) -> i128 {
        self.0
    }
    pub fn wrapping_add(self, o: Self) -> Self {
        Self::new(self.0.wrapping_add(o.0))
    }
}

/// The logic-valued vector, in its own module so the type is just
/// `Vec`: `logic::Vec<8>`. `std::vec::Vec` is untouched, because nothing
/// here is imported unqualified.
pub mod logic {
    use super::{Bit, Logic, U};

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct Vec<const N: usize>([Logic; N]);

    impl<const N: usize> Default for Vec<N> {
        fn default() -> Self {
            Vec([Logic::U; N])
        }
    }

    impl<const N: usize> Vec<N> {
        pub fn get(&self, i: usize) -> Logic {
            self.0[i]
        }
        pub fn set(&mut self, i: usize, v: Logic) {
            self.0[i] = v
        }
        pub fn all_defined(&self) -> bool {
            self.0.iter().all(|l| l.is_defined())
        }

        /// To a number, if every bit is defined. A design that ignores
        /// the `None` is a design that would have synthesised an X.
        pub fn to_u(&self) -> Option<U<N>> {
            let mut acc: u128 = 0;
            for i in (0..N).rev() {
                acc = (acc << 1) | (self.0[i].to_bit()?.to_bool() as u128);
            }
            Some(U::<N>::new(acc))
        }

        pub fn from_u(v: U<N>) -> Self {
            let mut out = [Logic::Zero; N];
            for i in 0..N {
                out[i] = Logic::from_bit(v.bit(i))
            }
            Vec(out)
        }

        pub fn resolve(&self, other: &Self) -> Self {
            let mut out = [Logic::U; N];
            for i in 0..N {
                out[i] = self.0[i].resolve(other.0[i])
            }
            Vec(out)
        }

        pub fn from_bits(bits: [Bit; N]) -> Self {
            let mut out = [Logic::Zero; N];
            for i in 0..N {
                out[i] = Logic::from_bit(bits[i])
            }
            Vec(out)
        }
    }
}

/// A value a trace can show: a fixed width and its bits as a string,
/// most significant first, in VCD's alphabet `0`, `1`, `x`, `z`.
/// Derived for a struct of values, most significant field first, and
/// for a fieldless enum, as the index of the variant.
pub trait Value: Copy {
    const WIDTH: usize;
    fn vcd(self) -> String;
    /// The named parts of a compound value, each with its width, its
    /// bits and, for an enum, the names of its variants, so a waveform
    /// can show a struct one field per signal. Empty for a scalar.
    fn parts(self) -> Vec<Part> {
        Vec::new()
    }
    /// The variants of an enum, by index, so a viewer can name a value
    /// rather than number it. None for anything else.
    fn names() -> Option<&'static [&'static str]> {
        None
    }
}

/// One field of a compound value in a trace.
pub struct Part {
    pub name: &'static str,
    pub width: usize,
    pub bits: String,
    pub names: Option<&'static [&'static str]>,
}

impl Value for Bit {
    const WIDTH: usize = 1;
    fn vcd(self) -> String {
        if self.to_bool() {
            "1".into()
        } else {
            "0".into()
        }
    }
}
impl Value for bool {
    const WIDTH: usize = 1;
    fn vcd(self) -> String {
        Bit::from_bool(self).vcd()
    }
}
impl Value for Logic {
    const WIDTH: usize = 1;
    fn vcd(self) -> String {
        match self {
            Logic::Zero | Logic::L => "0",
            Logic::One | Logic::H => "1",
            Logic::Z => "z",
            _ => "x",
        }
        .into()
    }
}
impl<const N: usize> Value for U<N> {
    const WIDTH: usize = N;
    fn vcd(self) -> String {
        format!("{:0width$b}", self.0, width = N)
    }
}
impl<const N: usize> Value for I<N> {
    const WIDTH: usize = N;
    fn vcd(self) -> String {
        let mask = if N >= 128 {
            u128::MAX
        } else {
            (1u128 << N) - 1
        };
        format!("{:0width$b}", (self.0 as u128) & mask, width = N)
    }
}
impl<const N: usize> Value for logic::Vec<N> {
    const WIDTH: usize = N;
    fn vcd(self) -> String {
        (0..N).rev().map(|i| self.get(i).vcd()).collect()
    }
}

/// Marker: a struct that moves between units over a channel. Derived
/// with `#[derive(Transaction)]`, which also asks for `Copy` and
/// `Default` so the channel can hold one.
pub trait Transaction: Copy + Default {}

/// A bare word is a transaction. A struct of fields is the usual case,
/// and derives it.
impl<const N: usize> Transaction for U<N> {}
impl<const N: usize> Transaction for I<N> {}
impl Transaction for Bit {}

/// A synchronisation domain and its policy, as a type. How data moves:
/// whether the domain is elastic, and how many transactions it tracks
/// in flight. Which clock edge moves it is a [`crate::comp::Clock`],
/// and one clock may carry several tags.
pub trait Tag {
    /// Inject ready and valid across the domain.
    const HANDSHAKE: bool = false;
    /// Track this many transactions in flight; 0 for unlimited.
    const CAPACITY: usize = 0;
}

/// The tag a design gets when it names none.
pub struct Raw;
impl Tag for Raw {}
