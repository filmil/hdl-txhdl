// SPDX-License-Identifier: Apache-2.0
//! Probe: an impl generic over a shared role, with the struct's bounds
//! not restated (#1914). Expected to fail: Rust does not imply a
//! struct's `where` bounds in its impls, as it does a trait's bounds
//! on an associated type, so an impl over `D: IsData` must restate
//! `<D as IsData>::Repr: Copy + Default`. An impl written on the
//! marker, `impl<T: Copy + Default> .. Signal<Data<T>>`, the way every
//! impl in the tree is written, needs nothing more:
//!
//! ```text
//! error[E0277]: the trait bound `<D as IsData>::Repr:
//!        std::marker::Copy` is not satisfied
//! error[E0277]: the trait bound `<D as IsData>::Repr:
//!        std::default::Default` is not satisfied
//! ```
use txhdl::{named_markers, named_type_params};

named_markers!(pub Data);

#[named_type_params(reuse(Data))]
pub struct Wire<Data: Copy + Default> {
    pub value: Data,
}

impl<D: IsData> Wire<D> {
    pub fn get(&self) -> <D as IsData>::Repr {
        self.value
    }
}
