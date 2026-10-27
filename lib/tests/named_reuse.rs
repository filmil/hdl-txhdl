// SPDX-License-Identifier: Apache-2.0
//! Markers declared once and reused (#1914): `named_markers!` declares
//! `Data` and `Domain` with bound-free roles, and several structs in
//! one module take them with `#[named_type_params(reuse(..))]`, each
//! stating its own bounds in its `where` clause. Before #1914 the
//! second struct would have made `Data` and `IsData` again, and the
//! module would not compile (E0428). What `reuse` refuses is in the
//! probes `experiments/rust_embedding/probe_named_reuse_*.rs`.
use std::mem::size_of;
use txhdl::{named_markers, named_type_params};

named_markers!(pub Data, pub Domain);

/// A clock domain, for the default.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fast;

/// One struct on the shared markers, with a default.
#[named_type_params(reuse(Data, Domain))]
pub struct Wire<Data: Copy + Default, Domain = Fast> {
    pub value: Data,
    pub domain: std::marker::PhantomData<Domain>,
}

/// A second in the same module, with other bounds on `Data`.
#[named_type_params(reuse(Data))]
pub struct Pair<Data: Into<u64> + Copy, Other> {
    pub a: Data,
    pub b: Other,
}

/// A tuple struct with a `where` clause of its own already, and a
/// parameter that may be unsized.
#[named_type_params(reuse(Data))]
pub struct Boxed<Data: ?Sized + std::fmt::Debug, Tag>(pub Box<Data>, pub Tag)
where
    Tag: Copy;

#[test]
fn two_structs_share_one_set_of_markers() {
    let w: Wire<Data<u8>> = Wire {
        value: 3,
        domain: std::marker::PhantomData,
    };
    assert_eq!(w.value, 3u8);
    let p: Pair<Data<u32>, Other<&str>> = Pair { a: 7, b: "seven" };
    let wide: u64 = p.a.into();
    assert_eq!((wide, p.b), (7, "seven"));
    // The markers take no room, and one marker type serves both.
    assert_eq!(size_of::<Data<u64>>(), 0);
    let _same: (Wire<Data<u16>, Domain<Fast>>, Pair<Data<u16>, Other<()>>) = (
        Wire {
            value: 1,
            domain: std::marker::PhantomData,
        },
        Pair { a: 2, b: () },
    );
}

#[test]
fn a_tuple_struct_with_its_own_where_and_an_unsized_parameter() {
    let b: Boxed<Data<str>, Tag<u8>> = Boxed("unsized".into(), 1);
    assert_eq!((&*b.0, b.1), ("unsized", 1u8));
}

#[test]
fn the_structs_macro_still_names_every_parameter() {
    let p: Pair![Other = bool, Data = u8] = Pair { a: 1, b: true };
    assert!(p.b);
    let w: Wire![Data = i16] = Wire {
        value: -1,
        domain: std::marker::PhantomData,
    };
    assert_eq!(w.value, -1i16);
}

mod clauses {
    //! A braced struct with a `where` clause of its own, holding a
    //! parenthesized type, with and without a trailing comma: the
    //! reused bounds join it, before the body. And markers of mixed
    //! visibility in one `named_markers!`.
    use txhdl::{named_markers, named_type_params};

    named_markers!(pub(crate) Item, pub Step);

    #[named_type_params(reuse(Item, Step))]
    pub struct Apply<Item: Copy, Step>
    where
        Step: Fn(u8) -> u8,
    {
        pub item: Item,
        pub step: Step,
    }

    #[named_type_params(reuse(Item))]
    pub struct Hold<Item: Copy, F>
    where
        F: Fn(Item) -> Item,
    {
        pub item: Item,
        pub f: F,
    }

    #[test]
    fn a_where_clause_with_a_parenthesized_type_is_joined() {
        let a: Apply<Item<u8>, Step<fn(u8) -> u8>> = Apply {
            item: 2,
            step: |x| x + 1,
        };
        assert_eq!((a.step)(a.item), 3);
        let h: Hold<Item<u8>, F<fn(u8) -> u8>> = Hold {
            item: 4,
            f: |x| x * 2,
        };
        assert_eq!((h.f)(h.item), 8);
    }
}

/// The crate's derives on a struct that reuses a marker (#1936): the
/// bounds `reuse` writes into the `where` clause reach every impl the
/// derives write, or `Reg<Data>` would ask for a `Value` it was not
/// promised.
mod derived {
    use txhdl::comp::{Data, IsData, Reg};
    use txhdl::netlist::Fields;
    use txhdl::types::{Value, U};
    use txhdl::{named_type_params, Trace};

    #[named_type_params(reuse(Data))]
    #[derive(Trace, Default)]
    pub struct Held<Data: Value + Default + 'static> {
        pub r: Reg<Data>,
    }

    #[test]
    fn derives_keep_the_where_clause() {
        let h = Held::<Data<U<4>>>::default();
        assert_eq!(h.r.get(), U::<4>::default());
        assert_eq!(<Held<Data<U<4>>> as Fields>::NAMES, &["r"]);
    }
}

/// The where clause of a tuple struct, which comes after its fields,
/// reaches a derived impl too (#1936): without it `Transaction`, which
/// asks for `Copy`, would not hold for `Tag<T>`.
mod tuple {
    use txhdl::types::Transaction;

    #[derive(Clone, Copy, Default, txhdl::Transaction)]
    pub struct Tag<T>(pub T)
    where
        T: Copy + Default + 'static;

    fn is_transaction<X: Transaction>() {}

    #[test]
    fn tuple_where_clause_reaches_the_impl() {
        is_transaction::<Tag<u8>>();
    }
}

/// A bundle on a reused marker (#1936): `Link` writes `LaneHost`, the
/// other side, which needs the same where clause as `Lane`.
mod linked {
    use txhdl::comp::{link, Data, IsData, Rx, Tx};
    use txhdl::types::{Bit, Transaction, Value, U};
    use txhdl::{named_type_params, Link, Ports};

    #[named_type_params(reuse(Data))]
    #[derive(Ports, Link)]
    pub struct Lane<Data: Transaction + Value> {
        pub data: Rx<Data>,
        pub ack: Tx<Bit>,
    }

    #[test]
    fn the_other_side_keeps_the_where_clause() {
        let (host, lane): (LaneHost<Data<U<8>>>, Lane<Data<U<8>>>) =
            link::<Lane<Data<U<8>>>>();
        let _ends = (host, lane.data, lane.ack);
    }
}
