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
