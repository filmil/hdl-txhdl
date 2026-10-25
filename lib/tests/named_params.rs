// SPDX-License-Identifier: Apache-2.0
//! `#[named_type_params]` (#1899): what the macro rewrites and what it
//! leaves alone. A struct's fields resolve to the plain types the use
//! named, inside generics, arrays and tuples; a path's segment that
//! happens to share a parameter's name is not the parameter; a default
//! carries over as its marker; lifetimes, const parameters and a
//! `where` clause pass through. What it refuses is in the probes
//! `experiments/rust_embedding/probe_named_*.rs`.
use std::mem::size_of;
use txhdl::named_type_params;

/// A module whose type shares a parameter's name, `Burst`, to show a
/// path's segment is left as it is.
mod m {
    pub type Burst = u16;
}

#[named_type_params]
pub struct Bus<Addr: Copy + Into<u64>, Width: Copy, Burst> {
    pub addr: Addr,
    pub data: [Width; 2],
    pub queue: Vec<Burst>,
    pub pair: (Addr, Option<Width>),
    pub other: m::Burst,
}

#[named_type_params]
pub struct Window<'a, const N: usize, Item: Copy = u8>
where
    Item: Default,
{
    pub items: &'a [Item; N],
}

#[named_type_params]
pub struct Pair<Left, Right>(pub Left, pub Right);

type Axi = Bus<Addr<u32>, Width<u64>, Burst<u8>>;

/// Takes the plain types: it compiles only if the fields resolve.
fn plain(b: &Axi) -> (u32, u64, u8, u16) {
    (b.addr, b.data[1], b.queue[0], b.other)
}

#[test]
fn the_fields_are_the_types_the_use_named() {
    let b: Axi = Bus {
        addr: 7,
        data: [1, 2],
        queue: vec![3],
        pair: (8, Some(9)),
        other: 10,
    };
    assert_eq!(plain(&b), (7, 2, 3, 10));
    assert_eq!(b.pair, (8u32, Some(9u64)));
    // The markers take no room: the struct is its fields.
    assert_eq!(size_of::<Addr<u32>>(), 0);
    assert_eq!(
        size_of::<Pair<Left<u32>, Right<u8>>>(),
        size_of::<(u32, u8)>()
    );
}

#[test]
fn a_default_carries_over_and_the_rest_passes_through() {
    let items = [1u8, 2, 3];
    // `Item` left out: its default, `Item<u8>`.
    let w: Window<'_, 3> = Window { items: &items };
    assert_eq!(w.items[2], 3u8);
    let wide = [5u32; 2];
    let v: Window<'_, 2, Item<u32>> = Window { items: &wide };
    assert_eq!(v.items[0], 5u32);
}

#[test]
fn a_tuple_struct_too() {
    let p: Pair<Left<u32>, Right<&str>> = Pair(1, "one");
    assert_eq!((p.0, p.1), (1u32, "one"));
}

/// Two const parameters of the same type, the kind a swap of goes
/// unnoticed: the struct's macro gives them by name.
#[named_type_params]
pub struct Lanes<const WIDTH: usize, const COUNT: usize> {
    pub lanes: [[u8; WIDTH]; COUNT],
}

#[test]
fn the_structs_macro_gives_every_parameter_by_name() {
    // In any order; the macro writes them in the struct's.
    let b: Bus![Burst = u8, Width = u64, Addr = u32] = Bus {
        addr: 1,
        data: [2, 3],
        queue: vec![4],
        pair: (5, None),
        other: 6,
    };
    let same: Axi = b;
    assert_eq!(plain(&same), (1, 3, 4, 6));
    // A const parameter by name, the lifetime written for it, and a
    // default left out or given.
    let items = [7u8; 4];
    let w: Window![N = 4] = Window { items: &items };
    assert_eq!(w.items.len(), 4);
    let wide = [8u32; 2];
    let v: Window![Item = u32, N = 2] = Window { items: &wide };
    assert_eq!(v.items[1], 8u32);
    // Only const parameters: COUNT and WIDTH each where it belongs.
    let l: Lanes![COUNT = 3, WIDTH = 2] = Lanes { lanes: [[0; 2]; 3] };
    assert_eq!(l.lanes.len(), 3);
    assert_eq!(size_of::<Lanes![WIDTH = 2, COUNT = 3]>(), 6);
}

/// More shapes of field type: a function pointer, a reference, a slice,
/// a projection written `<P as Trait>::..`, and a field named like a
/// parameter, which keeps its name, after a field attribute, which is
/// left as it is.
#[named_type_params]
#[allow(non_snake_case)]
pub struct Shapes<'a, Arg: Copy, Ret, Src: IntoIterator> {
    pub f: fn(Arg) -> Ret,
    pub r: &'a Arg,
    pub s: &'a [Ret],
    pub first: Option<<Src as IntoIterator>::Item>,
    #[doc = "Named like the parameter `Ret`, and still a `u8`."]
    pub Ret: u8,
}

fn double(x: u32) -> u64 {
    2 * u64::from(x)
}

#[test]
fn fn_pointers_references_slices_and_projections() {
    let arg = 21u32;
    let rets = [1u64, 2];
    let s: Shapes<'_, Arg<u32>, Ret<u64>, Src<Vec<i8>>> = Shapes {
        f: double,
        r: &arg,
        s: &rets,
        first: Some(-1i8),
        Ret: 9,
    };
    assert_eq!(
        ((s.f)(*s.r), s.s[1], s.first, s.Ret),
        (42u64, 2u64, Some(-1i8), 9u8)
    );
}

/// A derive below the macro reaches the struct as rewritten: its
/// bounds on the renamed parameters are met by the markers, and the
/// fields' types meet the derived traits because the parameters' bounds
/// say so, and carry over to the roles. One above the macro would see
/// the struct as written, before the rewrite, and fail, so a derive
/// goes below it.
#[named_type_params]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point<
    X: Copy + std::fmt::Debug + Default + PartialEq,
    Y: Copy + std::fmt::Debug + Default + PartialEq,
> {
    pub x: X,
    pub y: Y,
}

#[test]
fn attributes_and_derives_reach_the_struct() {
    let p: Point<X<u8>, Y<i16>> = Point { x: 1, y: -2 };
    let q = p;
    assert_eq!(p, q);
    assert_eq!(Point::<X<u8>, Y<i16>>::default(), Point { x: 0, y: 0 });
    assert_eq!(format!("{:?}", p), "Point { x: 1, y: -2 }");
}

/// A lifetime by name, so the macro can be used where `'_` cannot, in
/// a type alias.
type Statics = Window!['a = 'static, N = 2, Item = u16];

#[test]
fn a_lifetime_by_name() {
    static TWO: [u16; 2] = [3, 4];
    let w: Statics = Window { items: &TWO };
    assert_eq!(w.items[1], 4u16);
}

mod wires {
    //! `interface!` with a `Signal` given its clock domain by name, and
    //! one whose marker is named by its path: the ends take the bare
    //! arguments.
    use txhdl::comp::{settle, Data, DefaultClock, Domain, Signal};
    use txhdl::interface;
    use txhdl::types::{Bit, U};

    interface! {
        Link {
            go: Signal<Data<Bit>, Domain<DefaultClock>>,
            word: Signal<txhdl::comp::Data<U<8>>>,
        }
        role Left { out go, in word }
        role Right { in go, out word }
    }

    #[test]
    fn a_domain_and_a_path_through_interface() {
        let (l, r) = Link::new();
        l.go.set(Bit::One);
        r.word.set(U::<8>::from(5u8));
        settle();
        assert_eq!((r.go.get(), l.word.get().raw()), (Bit::One, 5));
    }
}
