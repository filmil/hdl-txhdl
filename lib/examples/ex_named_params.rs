// SPDX-License-Identifier: Apache-2.0
//! Type parameters given by name: `#[named_type_params]` (#1899).
//!
//! Rust has no named type arguments, so a struct of two type
//! parameters with the same bounds, `Bus<A, D>`, takes `Bus<u64, u32>`
//! as readily as `Bus<u32, u64>`, and a swap compiles. On a struct,
//! `#[named_type_params]` makes a marker for each type parameter, named
//! for it, and every use names each argument with its marker:
//! `Bus<Addr<u32>, Word<u64>>`. A marker in the wrong place, or a bare
//! type where a marker belongs, is refused by the compiler. The fields
//! are still the plain types, and the markers take no room.
//!
//! Each such struct also has a macro of its own name, which takes
//! every parameter by name and in any order, `Bus![Word = u64, Addr =
//! u32]`, and refuses one given twice, one it does not have, or one
//! left out. That is how a const parameter is given by name, since
//! stable Rust cannot read a number out of a marker: `Lanes![COUNT = 4,
//! WIDTH = 8]` is `Lanes<8, 4>`.
//!
//! Two structs of one module that name a parameter alike share its
//! marker: `named_markers!(pub Addr, pub Word)` declares the markers
//! once, and each struct takes them with
//! `#[named_type_params(reuse(..))]`, its own bounds in its `where`
//! clause. Here `Bus` and `Beat` both take `Word`.
//!
//! The runtime's `Signal` is written this way: `Signal<Data<T>>` is a
//! wire of `T`, and `Signal<Data<T>, Domain<C>>` one in clock domain
//! `C`.
use txhdl::comp::{Data, DefaultClock, Domain, Member, Signal};
use txhdl::types::U;
use txhdl::{named_markers, named_type_params};

// begin{bus}
// The markers, once, for the two structs below.
named_markers!(pub Addr, pub Word);

/// An address and a data word, two numbers of overlapping bounds.
#[named_type_params(reuse(Addr, Word))]
pub struct Bus<Addr: Copy + Into<u64>, Word: Copy> {
    pub addr: Addr,
    pub data: [Word; 2],
}

/// A 32-bit address and 64-bit data, each named.
type Axi = Bus<Addr<u32>, Word<u64>>;

/// One word and whether it is the last of a burst: the same `Word`.
#[named_type_params(reuse(Word))]
pub struct Beat<Word: Copy> {
    pub data: Word,
    pub last: bool,
}

/// Two numbers of the same type, which only names keep apart.
#[named_type_params]
pub struct Lanes<const WIDTH: usize, const COUNT: usize> {
    pub lanes: [U<WIDTH>; COUNT],
}
// end{bus}

/// The fields are plain numbers: this takes a `u32` and a `u64`.
fn show(b: &Axi) -> String {
    let (a, d): (u32, u64) = (b.addr, b.data[0] + b.data[1]);
    format!("addr {a:#x}, data sum {d}")
}

fn main() {
    // The same type, its parameters by name and in another order.
    let b: Bus![Word = u64, Addr = u32] = Bus {
        addr: 0x4000_0000,
        data: [5, 6],
    };
    println!("{}", show(&b));
    let beat: Beat<Word<u64>> = Beat {
        data: b.data[1],
        last: true,
    };
    println!("beat {} last {}", beat.data, beat.last);
    let l: Lanes![COUNT = 4, WIDTH = 8] = Lanes {
        lanes: [U::<8>::from(3u8); 4],
    };
    println!(
        "{} lanes of {} bits",
        l.lanes.len(),
        <U<8> as txhdl::types::Value>::WIDTH
    );
    println!(
        "Bus is {} bytes, as its fields are; Addr<u32> is {}",
        std::mem::size_of::<Axi>(),
        std::mem::size_of::<Addr<u32>>()
    );
    // begin{signal}
    // A wire of eight bits in the default clock domain, then one with
    // the domain named too.
    let (out, inp) = <Signal<Data<U<8>>> as Member>::new().split();
    out.set(U::<8>::from(42u8));
    let w: Signal<Data<U<8>>, Domain<DefaultClock>> = Member::new();
    let (_o, _i) = w.split();
    // end{signal}
    println!("the wire reads {}", inp.get().raw());
}
