// SPDX-License-Identifier: Apache-2.0
//! What the derives write, checked by compiling it: a unit the derives
//! once refused is declared here, and the tests say what it means.
use txhdl::comp::{Clock, DefaultClock, Out, Reg, Unit};
use txhdl::netlist::Fields;
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

/// A const parameter with a default. `derive(Trace)` copied the default
/// into the `impl`s it wrote, where Rust refuses one (issue 1044).
#[derive(Trace, Default)]
pub struct Defaulted<const I: usize = 2> {
    pub idle: Reg<Bit>,
    pub id: Reg<U<I>>,
}

/// The same unit lowered, so that `#[lower]`'s code is checked against
/// a parameter with a default too.
#[lower]
impl<const I: usize> Unit for Defaulted<I> {
    async fn run(&mut self, _inp: (), busy: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            self.id.set(self.id.get() + 1);
            busy.set(!self.idle.get());
        }
    }
}

/// The width of the field `name` of a unit, as its `Fields` says.
fn width<T: Fields>(name: &str) -> usize {
    T::fields()
        .into_iter()
        .find(|f| f.0 == name)
        .map(|f| f.2)
        .expect("the field")
}

#[test]
fn a_const_default_is_what_the_type_alone_means() {
    // `Defaulted` alone is `Defaulted<2>`, and another width is still
    // a width of its own.
    let _: Defaulted = Defaulted::<2>::default();
    assert_eq!(<Defaulted as Fields>::NAMES, &["idle", "id"]);
    assert_eq!(width::<Defaulted>("id"), 2);
    assert_eq!(width::<Defaulted<1>>("id"), 1);
}

#[test]
fn a_unit_with_a_const_default_lowers() {
    // A default applies where a type is written, not to a path in an
    // expression, which Rust leaves to inference: so `<Defaulted>::`,
    // and not `Defaulted::`, is `Defaulted<2>` here.
    let net = <Defaulted>::lowered("defaulted");
    assert!(net.verilog().contains("module defaulted"));
}
