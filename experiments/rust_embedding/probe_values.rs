// Probe 15. The value types: our own, paralleling the primitives.
//
//   Bit        two valued. What synthesis maps.
//   Logic      nine valued, IEEE 1164 std_ulogic. What simulation needs,
//              because X and Z are how a design says "unknown" and
//              "nobody is driving".
//   U<N>, I<N> numeric vectors, unsigned and signed.
//   logic::Vec<N> a vector of Logic, for when X must propagate. It lives
//              in its own module so that `Vec` is the name it deserves:
//              `logic::Vec<8>` reads, and it never collides with
//              `std::vec::Vec`, which is only ever reached unqualified.
//
// Everything here compiles on stable. The one operation that does not is
// noted at the bottom, and it is the same `{A + B}` limit probe 1 found.

pub mod types {
    /// Two valued. `Default` is `Zero`, because a synthesised register
    /// comes out of reset at a defined value.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub enum Bit {
        #[default]
        Zero,
        One,
    }

    impl Bit {
        pub fn from_bool(b: bool) -> Self { if b { Bit::One } else { Bit::Zero } }
        pub fn to_bool(self) -> bool { matches!(self, Bit::One) }
        pub fn not(self) -> Self { if self.to_bool() { Bit::Zero } else { Bit::One } }
    }

    /// Nine valued, in IEEE 1164 order. `Default` is `U`: a signal that
    /// nobody has driven is uninitialised, not zero. That difference is
    /// the reason to have this type at all.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub enum Logic {
        #[default]
        U,          // uninitialised
        X,          // forcing unknown
        Zero,       // forcing 0
        One,        // forcing 1
        Z,          // high impedance
        W,          // weak unknown
        L,          // weak 0
        H,          // weak 1
        DontCare,   // don't care
    }

    impl Logic {
        const fn index(self) -> usize {
            match self {
                Logic::U => 0, Logic::X => 1, Logic::Zero => 2, Logic::One => 3,
                Logic::Z => 4, Logic::W => 5, Logic::L => 6, Logic::H => 7,
                Logic::DontCare => 8,
            }
        }

        const fn from_index(i: usize) -> Self {
            match i {
                0 => Logic::U, 1 => Logic::X, 2 => Logic::Zero, 3 => Logic::One,
                4 => Logic::Z, 5 => Logic::W, 6 => Logic::L, 7 => Logic::H,
                _ => Logic::DontCare,
            }
        }

        /// The IEEE 1164 resolution table. Two drivers on one wire give
        /// this. It is what makes a bus with several masters expressible
        /// without the language inventing a rule of its own.
        pub fn resolve(self, other: Logic) -> Logic {
            const T: [[usize; 9]; 9] = [
                [0,1,1,1,1,1,1,1,1],
                [1,1,1,1,1,1,1,1,1],
                [1,1,2,1,2,2,2,2,1],
                [1,1,1,3,3,3,3,3,1],
                [1,1,2,3,4,5,6,7,1],
                [1,1,2,3,5,5,5,5,1],
                [1,1,2,3,6,5,6,5,1],
                [1,1,2,3,7,5,5,7,1],
                [1,1,1,1,1,1,1,1,1],
            ];
            Logic::from_index(T[self.index()][other.index()])
        }

        /// Narrowing to two values. Fails where the value is not a
        /// definite 0 or 1, which is where a design would otherwise
        /// synthesise something it had not stated.
        pub fn to_bit(self) -> Option<Bit> {
            match self {
                Logic::Zero | Logic::L => Some(Bit::Zero),
                Logic::One | Logic::H => Some(Bit::One),
                _ => None,
            }
        }

        pub fn from_bit(b: Bit) -> Self {
            match b { Bit::Zero => Logic::Zero, Bit::One => Logic::One }
        }

        pub fn is_defined(self) -> bool { self.to_bit().is_some() }
    }

    /// An N-bit unsigned value.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub struct U<const N: usize>(u128);

    impl<const N: usize> U<N> {
        pub const WIDTH: usize = N;
        const MASK: u128 = if N >= 128 { u128::MAX } else { (1u128 << N) - 1 };

        pub const fn new(v: u128) -> Self { U(v & Self::MASK) }
        pub const fn raw(self) -> u128 { self.0 }

        pub fn bit(self, i: usize) -> Bit { Bit::from_bool((self.0 >> i) & 1 == 1) }

        /// Same-width arithmetic, wrapping. Stable: the result width is a
        /// standalone parameter.
        pub fn wrapping_add(self, o: Self) -> Self { Self::new(self.0.wrapping_add(o.0)) }
        pub fn wrapping_sub(self, o: Self) -> Self { Self::new(self.0.wrapping_sub(o.0)) }

        /// Resize to a stated width. `M` is standalone, so this is stable
        /// even though the width changes.
        pub fn resize<const M: usize>(self) -> U<M> { U::<M>::new(self.0) }

        /// A slice. Offset and width are const parameters, per probe 11.
        pub fn slice<const LO: usize, const LEN: usize>(self) -> U<LEN> {
            U::<LEN>::new(self.0 >> LO)
        }
    }

    /// An N-bit signed value, two's complement.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub struct I<const N: usize>(i128);

    impl<const N: usize> I<N> {
        pub const WIDTH: usize = N;

        pub fn new(v: i128) -> Self {
            let shift = 128 - N;
            I((v << shift) >> shift)   // sign extend from N bits
        }
        pub const fn raw(self) -> i128 { self.0 }
        pub fn wrapping_add(self, o: Self) -> Self { Self::new(self.0.wrapping_add(o.0)) }
    }

    /// The logic-valued vector. Its own module, so the type is just
    /// `Vec`: `logic::Vec<8>` says what it is without repeating the word.
    /// `std::vec::Vec` is unaffected, because nothing here is imported
    /// unqualified.
    /// The logic-valued vector. Its own module, so the type is just
    /// `Vec`: `logic::Vec<8>` says what it is without repeating the word.
    /// `std::vec::Vec` is unaffected, because nothing here is imported
    /// unqualified.
    pub mod logic {
        use super::{Logic, U};

        /// A vector of Logic, for when X and Z must propagate.
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub struct Vec<const N: usize>([Logic; N]);

        impl<const N: usize> Default for Vec<N> {
            fn default() -> Self { Vec([Logic::U; N]) }
        }

        impl<const N: usize> Vec<N> {
            pub fn get(&self, i: usize) -> Logic { self.0[i] }
            pub fn set(&mut self, i: usize, v: Logic) { self.0[i] = v }

            pub fn all_defined(&self) -> bool {
                self.0.iter().all(|l| l.is_defined())
            }

            /// To a number, if every bit is defined. A design that
            /// ignores the None is a design that would have synthesised
            /// an X.
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

            /// Resolution, bit by bit, for a wire with two drivers.
            pub fn resolve(&self, other: &Self) -> Self {
                let mut out = [Logic::U; N];
                for i in 0..N {
                    out[i] = self.0[i].resolve(other.0[i])
                }
                Vec(out)
            }
        }
    }

}

use types::*;

pub fn arithmetic() -> U<8> {
    U::<8>::new(250).wrapping_add(U::<8>::new(10))   // wraps to 4
}

pub fn widen(x: U<8>) -> U<32> { x.resize::<32>() }

pub fn field(w: U<32>) -> U<4> { w.slice::<12, 4>() }

pub fn signed() -> I<8> { I::<8>::new(-1).wrapping_add(I::<8>::new(1)) }

/// An undriven signal is U, not zero, and it stays unknown through a
/// conversion rather than quietly becoming a number. `logic::Vec` is
/// named without stutter and does not shadow `std::vec::Vec`, which this
/// function also uses, to show that the two coexist.
pub fn undriven_is_unknown() -> bool {
    let v = logic::Vec::<8>::default();
    let _also_a_vec: std::vec::Vec<u8> = vec![1, 2, 3];
    v.to_u().is_none() && !v.all_defined()
}

/// Two drivers on one wire.
pub fn contention() -> Logic {
    Logic::One.resolve(Logic::Zero)      // X
}

pub fn pullup() -> Logic {
    Logic::Z.resolve(Logic::H)           // H
}

// What does NOT compile here, and it is probe 1's limit again:
//
//   pub fn mul<const A: usize, const B: usize>(a: U<A>, b: U<B>) -> U<{A + B}>
//
// so a widening multiply is either a declared-width function per pair or
// a nightly build.
