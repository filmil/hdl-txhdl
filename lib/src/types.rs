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
    /// Low.
    #[default]
    Zero,
    /// High.
    One,
}

impl Bit {
    /// A bit from a truth value, so that a compare, which yields
    /// `bool`, can drive a signal.
    pub fn from_bool(b: bool) -> Self {
        if b {
            Bit::One
        } else {
            Bit::Zero
        }
    }
    /// The other way: a bit as a condition an `if` can take.
    pub fn to_bool(self) -> bool {
        matches!(self, Bit::One)
    }
    /// The bit as an `M`-bit value, `0` or `1`: what a compare yields
    /// into a datapath. Lowered as the bare compare; the target's width
    /// extends it.
    pub fn zext<const M: usize>(self) -> U<M> {
        U::from(self.to_bool() as u8)
    }
}

// A condition is a `Bit` or a `bool`, and either converts to the other:
// a compare yields a `bool`, a wire holds a `Bit`, and `when!`, `mux`
// and a register's `set` take both. The logic operators are `&`, `|`,
// `^` and `!`, on a `Bit` or across the two, and the result is a `Bit`.
// `&&` and `||` cannot be overloaded, so they stay `bool` only.
impl From<bool> for Bit {
    fn from(b: bool) -> Self {
        Self::from_bool(b)
    }
}
impl From<Bit> for bool {
    fn from(b: Bit) -> bool {
        b.to_bool()
    }
}
impl std::ops::Not for Bit {
    type Output = Bit;
    fn not(self) -> Bit {
        Self::from_bool(!self.to_bool())
    }
}
macro_rules! bit_ops {
    ($($tr:ident $f:ident $op:tt),*) => { $(
        impl<R: Into<Bit>> std::ops::$tr<R> for Bit {
            type Output = Bit;
            fn $f(self, o: R) -> Bit {
                Bit::from_bool(self.to_bool() $op o.into().to_bool())
            }
        }
        impl std::ops::$tr<Bit> for bool {
            type Output = Bit;
            fn $f(self, o: Bit) -> Bit {
                Bit::from_bool(self $op o.to_bool())
            }
        }
    )* };
}
bit_ops!(BitAnd bitand &, BitOr bitor |, BitXor bitxor ^);

/// Nine valued, in IEEE 1164 order. `Default` is `U`: a signal nobody
/// has driven is uninitialised, not zero. That difference is the reason
/// the type exists.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Logic {
    /// Uninitialised: nobody has driven this yet.
    #[default]
    U,
    /// Unknown, and strongly driven: two drivers disagree.
    X,
    /// Driven low.
    Zero,
    /// Driven high.
    One,
    /// High impedance: nobody is driving, on a bus that allows it.
    Z,
    /// Unknown, and weakly driven: two weak drivers disagree.
    W,
    /// Weakly low, a pull-down.
    L,
    /// Weakly high, a pull-up.
    H,
    /// Any value will do, for a synthesiser to choose.
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

    /// A two-valued bit as a nine-valued one, strongly driven.
    pub fn from_bit(b: Bit) -> Self {
        match b {
            Bit::Zero => Logic::Zero,
            Bit::One => Logic::One,
        }
    }

    /// Whether this is a definite zero or one, strongly or weakly.
    /// Uninitialised, unknown, floating and don't-care are not.
    pub fn is_defined(self) -> bool {
        self.to_bit().is_some()
    }
}

/// The widest value one limb holds: a `U<N>` keeps its bits in one Rust
/// integer of this many, and an `I<N>` is at most this wide. A wider
/// unsigned value is `U<N, L>`, in `L` limbs of this many (issue 503).
pub const MAX_WIDTH: usize = 128;

/// An N-bit unsigned value, kept in `L` limbs of 128 bits, least
/// significant first.
///
/// `L` has a default of one, so `U<N>` is a value of at most 128 bits
/// and is what it always was; a wider one names its limbs, `U<256, 2>`,
/// since stable Rust cannot size the array from `N` itself (issue 503,
/// and `probe_limbs`). `L` must be the number of limbs `N` needs, which
/// `WIDTH` checks where it is first read.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct U<const N: usize, const L: usize = 1>([u128; L]);

impl<const N: usize, const L: usize> Default for U<N, L> {
    fn default() -> Self {
        Self::new(0)
    }
}

/// As it always printed for one limb, `U(5)`, and the limbs for more.
impl<const N: usize, const L: usize> std::fmt::Debug for U<N, L> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if L == 1 {
            f.debug_tuple("U").field(&self.0[0]).finish()
        } else {
            f.debug_tuple("U").field(&self.0).finish()
        }
    }
}

/// Numeric order, which for more than one limb is the top limb first.
impl<const N: usize, const L: usize> Ord for U<N, L> {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        for i in (0..L).rev() {
            match self.0[i].cmp(&o.0[i]) {
                std::cmp::Ordering::Equal => continue,
                other => return other,
            }
        }
        std::cmp::Ordering::Equal
    }
}
impl<const N: usize, const L: usize> PartialOrd for U<N, L> {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

impl<const N: usize, const L: usize> U<N, L> {
    /// The width in bits, which is `N`. A derive reads it to lay a
    /// compound value out, so every value type has one, and reading it
    /// is where a width its limbs cannot hold stops the build.
    pub const WIDTH: usize = {
        assert!(
            L >= 1 && N <= MAX_WIDTH * L && (L == 1 || N > MAX_WIDTH * (L - 1)),
            "a U<N> is at most 128 bits wide: a wider value is U<N, L>, \
             with L the number of 128-bit limbs its bits need, \
             (N + 127) / 128"
        );
        N
    };
    /// The bits the top limb keeps.
    const TOP_MASK: u128 = {
        let top = Self::WIDTH - MAX_WIDTH * (L - 1);
        if top == MAX_WIDTH {
            u128::MAX
        } else {
            (1u128 << top) - 1
        }
    };

    const fn masked(mut l: [u128; L]) -> Self {
        l[L - 1] &= Self::TOP_MASK;
        U(l)
    }

    /// A value from its bits, truncated to `N` of them. Anything
    /// above the width is dropped rather than refused, which is what
    /// a register of `N` bits does with a wider number.
    pub const fn new(v: u128) -> Self {
        let mut l = [0u128; L];
        l[0] = v;
        Self::masked(l)
    }
    /// A value from its limbs, least significant first, truncated to
    /// `N` bits: how a value wider than 128 bits is built.
    pub const fn from_limbs(l: [u128; L]) -> Self {
        Self::masked(l)
    }
    /// The limbs, least significant first.
    pub const fn limbs(self) -> [u128; L] {
        self.0
    }
    /// The bits as a plain integer, for a testbench to print or
    /// compare: the low 128 of them, which for `U<N>` is all of them.
    /// Inside a lowered unit this reads as the value itself, so it
    /// costs nothing in the netlist.
    pub const fn raw(self) -> u128 {
        self.0[0]
    }
    /// One bit of it, counting from zero at the least significant.
    pub fn bit(self, i: usize) -> Bit {
        let (l, b) = (i / MAX_WIDTH, i % MAX_WIDTH);
        Bit::from_bool(l < L && (self.0[l] >> b) & 1 == 1)
    }

    /// A multiply with the result width stated: `a.mul::<64>(b)`. The
    /// width is a standalone parameter, so this is stable; `U<{A + B}>`
    /// would not be. The product is at most 128 bits wide.
    ///
    /// `*` is `std::ops::Mul`, which answers in the width it was given,
    /// wrapping; this is the multiply whose product is wider, so it keeps
    /// the operation's name rather than the trait's.
    #[allow(clippy::should_implement_trait)]
    pub fn mul<const M: usize>(self, o: impl Into<Self>) -> U<M> {
        let o = o.into();
        if L == 1 {
            return U::<M>::new(self.0[0].wrapping_mul(o.0[0]));
        }
        U::<M>::new(limbs::mul(&self.0, &o.0)[0])
    }

    /// Resize to a stated width, of at most 128 bits. `M` is standalone,
    /// so stable.
    pub fn resize<const M: usize>(self) -> U<M> {
        U::<M>::new(self.0[0])
    }

    /// A slice, of at most 128 bits, which is how a word comes out of a
    /// wide value. Offset and width are const parameters; an offset may
    /// also be a run-time value through [`U::slice_at`], but a width may
    /// not, because the width is the type.
    pub fn slice<const LO: usize, const LEN: usize>(self) -> U<LEN> {
        self.slice_at::<LEN>(LO)
    }

    /// `LEN` bits starting at `lo`, where `lo` is decided at run
    /// time rather than in the type. `LEN` is still a parameter,
    /// because the width of the result is the width of a wire.
    pub fn slice_at<const LEN: usize>(self, lo: usize) -> U<LEN> {
        if L == 1 {
            return U::<LEN>::new(self.0[0] >> lo);
        }
        U::<LEN>::new(limbs::shr(&self.0, lo)[0])
    }
}

/// Arithmetic across limbs, least significant first, for a value wider
/// than one. A value of one limb never comes here: every operator has
/// its one-`u128` path first, so a narrow value costs what it always
/// did (issue 503).
// Each loop walks two arrays of limbs in step, by index, which reads
// plainer than zipped iterators with a carry threaded through.
#[allow(clippy::needless_range_loop)]
mod limbs {
    use super::MAX_WIDTH;

    pub fn shl<const L: usize>(a: &[u128; L], k: usize) -> [u128; L] {
        let (w, b) = (k / MAX_WIDTH, k % MAX_WIDTH);
        let mut r = [0u128; L];
        for i in (w..L).rev() {
            let s = i - w;
            let mut v = a[s] << b;
            if b > 0 && s > 0 {
                v |= a[s - 1] >> (MAX_WIDTH - b);
            }
            r[i] = v;
        }
        r
    }

    pub fn shr<const L: usize>(a: &[u128; L], k: usize) -> [u128; L] {
        let (w, b) = (k / MAX_WIDTH, k % MAX_WIDTH);
        let mut r = [0u128; L];
        for i in 0..L.saturating_sub(w) {
            let s = i + w;
            let mut v = a[s] >> b;
            if b > 0 && s + 1 < L {
                v |= a[s + 1] << (MAX_WIDTH - b);
            }
            r[i] = v;
        }
        r
    }

    /// The sum, and whether it carried out of the top.
    pub fn add<const L: usize>(
        a: &[u128; L],
        b: &[u128; L],
    ) -> ([u128; L], bool) {
        let mut r = [0u128; L];
        let mut carry = false;
        for i in 0..L {
            let (s, c1) = a[i].overflowing_add(b[i]);
            let (s, c2) = s.overflowing_add(carry as u128);
            r[i] = s;
            carry = c1 || c2;
        }
        (r, carry)
    }

    pub fn sub<const L: usize>(a: &[u128; L], b: &[u128; L]) -> [u128; L] {
        let mut r = [0u128; L];
        let mut borrow = false;
        for i in 0..L {
            let (s, b1) = a[i].overflowing_sub(b[i]);
            let (s, b2) = s.overflowing_sub(borrow as u128);
            r[i] = s;
            borrow = b1 || b2;
        }
        r
    }

    /// The product's low `L` limbs, by 64-bit digits.
    pub fn mul<const L: usize>(a: &[u128; L], b: &[u128; L]) -> [u128; L] {
        let digits = |x: &[u128; L]| -> Vec<u64> {
            x.iter()
                .flat_map(|v| [*v as u64, (*v >> 64) as u64])
                .collect()
        };
        let (da, db) = (digits(a), digits(b));
        let n = 2 * L;
        let mut acc = vec![0u64; n];
        for i in 0..n {
            let mut carry = 0u128;
            for j in 0..n - i {
                let t =
                    acc[i + j] as u128 + da[i] as u128 * db[j] as u128 + carry;
                acc[i + j] = t as u64;
                carry = t >> 64;
            }
        }
        let mut r = [0u128; L];
        for (k, v) in r.iter_mut().enumerate() {
            *v = acc[2 * k] as u128 | (acc[2 * k + 1] as u128) << 64;
        }
        r
    }

    pub fn is_zero<const L: usize>(a: &[u128; L]) -> bool {
        a.iter().all(|v| *v == 0)
    }

    pub fn ge<const L: usize>(a: &[u128; L], b: &[u128; L]) -> bool {
        for i in (0..L).rev() {
            if a[i] != b[i] {
                return a[i] > b[i];
            }
        }
        true
    }

    /// `a % b` for `b` not zero, bit by bit from the top of `n` bits: the
    /// running remainder is shifted up, the bit carried out of the top
    /// kept, and `b` taken away whenever what is there reaches it.
    pub fn rem<const L: usize>(
        a: &[u128; L],
        b: &[u128; L],
        n: usize,
    ) -> [u128; L] {
        let mut r = [0u128; L];
        for i in (0..n).rev() {
            let out = (r[L - 1] >> (MAX_WIDTH - 1)) & 1 == 1;
            r = shl(&r, 1);
            r[0] |= (a[i / MAX_WIDTH] >> (i % MAX_WIDTH)) & 1;
            if out || ge(&r, b) {
                r = sub(&r, b);
            }
        }
        r
    }
}

// Conversions from the primitive integers, so a value is written as a
// number and the width comes from the context. `i32` is included because
// an integer literal with no other constraint is an `i32`, and a literal
// is the common case; a negative one is a bug, and is caught in debug.
macro_rules! from_int {
    ($($t:ty),*) => { $(
        impl<const N: usize, const L: usize> From<$t> for U<N, L> {
            fn from(v: $t) -> Self { Self::new(v as u128) }
        }
    )* };
}
from_int!(u8, u16, u32, u64, u128, usize);

/// A value as an address: what a memory's `read` and `at` take.
impl<const N: usize, const L: usize> From<U<N, L>> for usize {
    fn from(v: U<N, L>) -> usize {
        v.0[0] as usize
    }
}

impl<const N: usize, const L: usize> From<i32> for U<N, L> {
    fn from(v: i32) -> Self {
        debug_assert!(v >= 0, "a negative literal into an unsigned U<{N}>");
        Self::new(v as u128)
    }
}

// The operators of a datapath are Rust's operators. `+`, `-`, `*` and
// unary `-` are same-width and wrapping, and `%` is the remainder,
// which refuses zero as the netlist does; `&`, `|` and `^` are
// bitwise; `!` is the complement; `<<` and `>>` are logical shifts by
// an amount that is an integer, not a value. The right operand of an
// arithmetic or bitwise operator is anything that converts, so a
// literal is written as a literal: `n + 1`, `flags & 0xF`. A compare,
// `==`, `!=`, `<`, `<=`, `>`, `>=`, is unsigned, yields a `bool`, and
// takes a literal on the right too: `count == 8`. Every one lowers to
// the operator of the same name, and every one works across limbs for
// a value wider than 128 bits, a limb at a time. What has no operator
// is a method: `sra`, the arithmetic shift, `lt_signed`, the signed
// compare, `mul::<M>`, the widening multiply, `concat::<_, M>`,
// `sext::<M>` and `zext::<M>`, each with a width that is the sum of two
// others stated, because that sum needs nightly Rust to write.
macro_rules! u_ops {
    ($($tr:ident $f:ident |$a:ident, $b:ident| $one:expr, $wide:expr),*) => { $(
        impl<const N: usize, const L: usize, R: Into<U<N, L>>> std::ops::$tr<R>
            for U<N, L>
        {
            type Output = U<N, L>;
            fn $f(self, o: R) -> U<N, L> {
                let o = o.into();
                if L == 1 {
                    let ($a, $b) = (self.0[0], o.0[0]);
                    return U::<N, L>::new($one);
                }
                let ($a, $b) = (&self.0, &o.0);
                U::<N, L>::masked($wide)
            }
        }
    )* };
}
u_ops!(
    Add add |a, b| a.wrapping_add(b), limbs::add(a, b).0,
    Sub sub |a, b| a.wrapping_sub(b), limbs::sub(a, b),
    BitAnd bitand |a, b| a & b, std::array::from_fn(|i| a[i] & b[i]),
    BitOr bitor |a, b| a | b, std::array::from_fn(|i| a[i] | b[i]),
    BitXor bitxor |a, b| a ^ b, std::array::from_fn(|i| a[i] ^ b[i]),
    Mul mul |a, b| a.wrapping_mul(b), limbs::mul(a, b),
    Rem rem |a, b| {
        assert!(b != 0, "a remainder by zero, which the netlist refuses too");
        a % b
    }, {
        assert!(
            !limbs::is_zero(b),
            "a remainder by zero, which the netlist refuses too"
        );
        limbs::rem(a, b, N)
    }
);
/// Negation, wrapping: the two's complement at the same width, as
/// `0 - x` is and as the netlist's `~x + 1` is (issue 496).
impl<const N: usize, const L: usize> std::ops::Neg for U<N, L> {
    type Output = U<N, L>;
    fn neg(self) -> U<N, L> {
        U::<N, L>::new(0) - self
    }
}
impl<const N: usize, const L: usize> std::ops::Not for U<N, L> {
    type Output = U<N, L>;
    fn not(self) -> U<N, L> {
        U::<N, L>::masked(self.0.map(|v| !v))
    }
}
macro_rules! u_shifts {
    ($($t:ty),*) => { $(
        impl<const N: usize, const L: usize> std::ops::Shl<$t> for U<N, L> {
            type Output = U<N, L>;
            fn shl(self, k: $t) -> U<N, L> {
                let k = k as usize;
                if k >= N {
                    U::<N, L>::new(0)
                } else if L == 1 {
                    U::<N, L>::new(self.0[0] << k)
                } else {
                    U::<N, L>::masked(limbs::shl(&self.0, k))
                }
            }
        }
        impl<const N: usize, const L: usize> std::ops::Shr<$t> for U<N, L> {
            type Output = U<N, L>;
            fn shr(self, k: $t) -> U<N, L> {
                let k = k as usize;
                if k >= N {
                    U::<N, L>::new(0)
                } else if L == 1 {
                    U::<N, L>::new(self.0[0] >> k)
                } else {
                    U::<N, L>::masked(limbs::shr(&self.0, k))
                }
            }
        }
    )* };
}
u_shifts!(usize, u8, u32, i32);
macro_rules! u_compare {
    ($($t:ty),*) => { $(
        impl<const N: usize, const L: usize> PartialEq<$t> for U<N, L> {
            fn eq(&self, o: &$t) -> bool {
                *self == U::<N, L>::from(*o)
            }
        }
        impl<const N: usize, const L: usize> PartialOrd<$t> for U<N, L> {
            fn partial_cmp(&self, o: &$t) -> Option<std::cmp::Ordering> {
                Some(self.cmp(&U::<N, L>::from(*o)))
            }
        }
    )* };
}
u_compare!(u8, u16, u32, u64, u128, usize, i32);

impl<const N: usize, const L: usize> U<N, L> {
    /// An arithmetic shift right: the top bit fills in.
    pub fn sra(self, k: usize) -> Self {
        let k = k.min(N);
        let top = self.bit(N - 1).to_bool();
        let shifted = self >> k;
        if !top || k == 0 {
            return shifted;
        }
        // Ones in the `k` bits the shift emptied, at the top.
        let ones = !U::<N, L>::new(0);
        shifted | (ones << (N - k))
    }
    /// `self` above `low`: `M` is `N + K`, stated, and at most 128;
    /// `K` is `low`'s own width, and may be left to Rust as `_`.
    pub fn concat<const K: usize, const M: usize>(self, low: U<K>) -> U<M> {
        let high = if K >= MAX_WIDTH { 0 } else { self.0[0] << K };
        U::<M>::new(high | low.0[0])
    }
    /// Sign extension to `M` bits, at most 128.
    pub fn sext<const M: usize>(self) -> U<M> {
        let top = self.bit(N - 1).to_bool();
        if top && M > N {
            let ones = ((1u128 << (M - N)) - 1) << N;
            U::<M>::new(self.0[0] | ones)
        } else {
            U::<M>::new(self.0[0])
        }
    }
    /// Zero extension or truncation; `resize` by another name.
    pub fn zext<const M: usize>(self) -> U<M> {
        self.resize::<M>()
    }
    /// Signed less-than, both as two's complement of `N` bits.
    pub fn lt_signed(self, o: Self) -> Bit {
        Bit::from_bool(self.to_i().raw() < o.to_i().raw())
    }
    /// The same bits read as two's complement, for a value of at most
    /// 128 bits. The bits do not move; only what they are taken to mean
    /// does.
    pub fn to_i(self) -> I<N> {
        I::<N>::new(self.0[0] as i128)
    }
    /// The same bits back again, read as unsigned.
    pub fn from_i(v: I<N>) -> Self {
        Self::new(v.raw() as u128)
    }
}

/// An N-bit signed value, two's complement, `N` at most
/// [`MAX_WIDTH`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct I<const N: usize>(i128);

impl<const N: usize> Default for I<N> {
    fn default() -> Self {
        Self::new(0)
    }
}

impl<const N: usize> I<N> {
    /// The width in bits, which is `N`, checked as `U`'s is.
    pub const WIDTH: usize = {
        assert!(N <= MAX_WIDTH, "an I<N> is at most 128 bits wide");
        N
    };

    /// A value from a number, sign extended into `N` bits: what does
    /// not fit is dropped and the top bit of what remains becomes the
    /// sign, which is what a register of `N` bits holds.
    pub fn new(v: i128) -> Self {
        let shift = MAX_WIDTH - Self::WIDTH;
        I((v << shift) >> shift)
    }
    /// The value as a plain signed integer.
    pub const fn raw(self) -> i128 {
        self.0
    }
}
impl<const N: usize> std::ops::Add for I<N> {
    type Output = I<N>;
    fn add(self, o: I<N>) -> I<N> {
        Self::new(self.0.wrapping_add(o.0))
    }
}

/// The logic-valued vector, in its own module so the type is just
/// `Vec`: `logic::Vec<8>`. `std::vec::Vec` is untouched, because nothing
/// here is imported unqualified.
pub mod logic {
    use super::{Bit, Logic, U};

    /// `N` nine-valued bits, least significant first: a bus whose
    /// wires may be undriven or contended, which `U<N>` cannot say.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct Vec<const N: usize>([Logic; N]);

    impl<const N: usize> Default for Vec<N> {
        fn default() -> Self {
            Vec([Logic::U; N])
        }
    }

    impl<const N: usize> Vec<N> {
        /// One bit of it, counting from zero at the least
        /// significant.
        pub fn get(&self, i: usize) -> Logic {
            self.0[i]
        }
        /// Drive one bit of it.
        pub fn set(&mut self, i: usize, v: Logic) {
            self.0[i] = v
        }
        /// Whether every bit is a definite zero or one. A vector
        /// that is not is one no number can be made of.
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

        /// From a number: every bit strongly driven, so nothing is
        /// uninitialised or floating.
        pub fn from_u(v: U<N>) -> Self {
            let mut out = [Logic::Zero; N];
            for (i, o) in out.iter_mut().enumerate() {
                *o = Logic::from_bit(v.bit(i))
            }
            Vec(out)
        }

        /// What two drivers on one wire come to, bit by bit, by the
        /// IEEE 1164 table: two that disagree strongly give `X`.
        pub fn resolve(&self, other: &Self) -> Self {
            let mut out = [Logic::U; N];
            for (i, o) in out.iter_mut().enumerate() {
                *o = self.0[i].resolve(other.0[i])
            }
            Vec(out)
        }

        /// From two-valued bits, each strongly driven.
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
    /// How many bits the value occupies on a wire. A lowering uses
    /// it to size a port, and a derive sums it over a struct's
    /// fields.
    const WIDTH: usize;
    /// The bits, most significant first, in VCD's alphabet.
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
    /// The fields of a compound value, each with its width, the first
    /// field highest, so a lowering can slice one out of the whole.
    /// Empty for a scalar.
    fn layout() -> Vec<(&'static str, usize)> {
        Vec::new()
    }
}

/// One field of a compound value in a trace.
pub struct Part {
    /// The field's name, which becomes the signal's name under the
    /// value's own scope.
    pub name: &'static str,
    /// How many bits it occupies.
    pub width: usize,
    /// Its bits, most significant first, in VCD's alphabet.
    pub bits: String,
    /// For an enum, its variants by index, so a viewer can name the
    /// value rather than number it. None for anything else.
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
impl<const N: usize, const L: usize> Value for U<N, L> {
    const WIDTH: usize = U::<N, L>::WIDTH;
    fn vcd(self) -> String {
        if L == 1 {
            return format!("{:0width$b}", self.0[0], width = N);
        }
        // The top limb holds what is left of `N`, and the rest are whole.
        let top = N - MAX_WIDTH * (L - 1);
        let mut s = format!("{:0width$b}", self.0[L - 1], width = top);
        for i in (0..L - 1).rev() {
            s.push_str(&format!("{:0128b}", self.0[i]));
        }
        s
    }
}
impl<const N: usize> Value for I<N> {
    const WIDTH: usize = I::<N>::WIDTH;
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
pub trait Transaction: Copy + Default + 'static {}

/// A bare word is a transaction. A struct of fields is the usual case,
/// and derives it.
impl<const N: usize, const L: usize> Transaction for U<N, L> {}
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

/// Values wider than one limb (issue 503), against answers worked out
/// by hand, and against the one-limb arithmetic where both apply.
#[cfg(test)]
mod wide_tests {
    use super::{Value, U};

    type W = U<256, 2>;
    const TOP: u128 = u128::MAX;

    #[test]
    fn a_sum_carries_into_the_next_limb() {
        let x = W::from_limbs([TOP, 0]) + 1u8;
        assert_eq!(x.limbs(), [0, 1]);
        let back = x - 1u8;
        assert_eq!(back.limbs(), [TOP, 0], "and the difference borrows back");
    }

    #[test]
    fn the_top_limb_keeps_only_the_width() {
        // 200 bits: the top limb holds 72 of them.
        let x = U::<200, 2>::from_limbs([TOP, TOP]);
        assert_eq!(x.limbs(), [TOP, (1u128 << 72) - 1]);
        assert_eq!((x + 1u8).limbs(), [0, 0], "and wraps at 200 bits");
        assert_eq!(<U<200, 2> as Value>::WIDTH, 200);
        assert_eq!(x.vcd().len(), 200);
        assert!(x.vcd().chars().all(|c| c == '1'));
    }

    #[test]
    fn a_product_lands_in_the_top_limb() {
        // 2^128 * 2^64 = 2^192.
        let x = W::from_limbs([0, 1]) * W::from_limbs([1u128 << 64, 0]);
        assert_eq!(x.limbs(), [0, 1u128 << 64]);
    }

    #[test]
    fn a_remainder_across_limbs() {
        // 2^200 + 5, and 2^3 is 1 mod 7, so 2^200 = 2^2 = 4 mod 7.
        let x = W::from_limbs([5, 1u128 << 72]);
        assert_eq!((x % 7u8).limbs(), [2, 0]);
    }

    #[test]
    fn shifts_move_bits_across_the_limbs() {
        let x = W::from_limbs([1u128 << 127, 0]) << 1usize;
        assert_eq!(x.limbs(), [0, 1]);
        assert_eq!((x >> 1usize).limbs(), [1u128 << 127, 0]);
        assert!((W::from(1u8) << 255usize).bit(255).to_bool());
        assert_eq!((W::from(1u8) << 256usize).limbs(), [0, 0]);
    }

    #[test]
    fn the_order_is_numeric_top_limb_first() {
        let small = W::from_limbs([TOP, 0]);
        let big = W::from_limbs([0, 1]);
        assert!(small < big);
        assert!(big > 5u8);
        assert!(W::from(5u8) == 5u8);
    }

    #[test]
    fn a_word_comes_out_of_a_wide_value() {
        let x = W::from_limbs([0x1111, 0xabcd]);
        assert_eq!(x.slice::<128, 16>().raw(), 0xabcd);
        assert_eq!(x.slice::<120, 16>().raw(), 0xcd00);
        assert_eq!(x.slice::<0, 16>().raw(), 0x1111);
    }

    /// Where both apply, the limbs agree with one `u128`.
    #[test]
    fn narrow_operands_agree_with_one_limb() {
        let pairs: [(u128, u128); 5] = [
            (7, 3),
            (u64::MAX as u128, 12345),
            (1 << 100, (1 << 99) + 17),
            (0xdead_beef, 1),
            (99, 250),
        ];
        for (a, b) in pairs {
            let (wa, wb) = (W::from(a), W::from(b));
            let (na, nb) = (U::<128>::from(a), U::<128>::from(b));
            assert_eq!((wa & wb).raw(), (na & nb).raw());
            assert_eq!((wa | wb).raw(), (na | nb).raw());
            assert_eq!((wa ^ wb).raw(), (na ^ nb).raw());
            assert_eq!((wa % wb).raw(), (na % nb).raw(), "{a} % {b}");
            assert_eq!((wa + wb).raw(), (na + nb).raw());
            assert_eq!(wa < wb, na < nb);
        }
    }
}
