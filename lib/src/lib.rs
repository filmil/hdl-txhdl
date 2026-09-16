// SPDX-License-Identifier: Apache-2.0
//! TxHDL, embedded in Rust. A prototypical runtime.
//!
//! Every construct the language still needs, once everything Rust
//! already provides has been deleted. Four modules and two macros:
//!
//! - [`types`]: `Bit`, `Logic`, `U<N>`, `I<N>`, `logic::Vec<N>`,
//!   the `Transaction` marker and the `Tag` policy.
//! - [`comp`]: units, wires and their ends, interfaces, clocks,
//!   configurations, `join2` and `parallel!`, `mux`, and the executor.
//! - [`pipeline`]: operators that may take a cycle. All `async`.
//! - [`funcs`]: operators that cannot. All plain `fn`.
//! - `#[derive(Transaction)]`, `#[derive(Bus)]`, `#[derive(Value)]`,
//!   `#[derive(Trace)]`, `interface!`, `when!` and `case!`, re-exported
//!   from `txhdl_macros`; and `select!`, a value chosen by pattern,
//!   defined here.
//! - [`comp::trace`]: names for signals and a VCD writer.
//! - [`netlist`]: a structural Verilog skeleton from the same walk.
//!
//! This is a simulation-shaped prototype: values exist while it runs.
//! The lowering to hardware is the experiment the article states, and
//! this crate is what that experiment would be written against.

// Every public item carries its own documentation, and the build
// refuses one that does not. A type parameter is not covered by this
// lint, so a generic item says in prose what each of its parameters
// means and why any width that looks computable is stated instead.
#![deny(missing_docs)]

pub mod comp;
pub mod foreign;
pub mod funcs;
pub mod netlist;
pub mod pipeline;
pub mod types;

pub use txhdl_macros::{
    case, interface, lite_bridge, lower, pipeline, router, station, when, with,
    Bus, Trace, Transaction, Value,
};

/// `select!(value => { pattern => expr, .., _ => expr })`: a value
/// chosen by the first pattern that matches, `match` by another name.
/// The name says what the hardware is: a chain of multiplexers, the
/// arms in priority order, which is what `#[lower]` makes of it. A
/// pattern may carry an `if` guard, as `case!`'s may.
#[macro_export]
macro_rules! select {
    ($v:expr => { $($p:pat $(if $g:expr)? => $e:expr),+ $(,)? }) => {
        match $v { $($p $(if $g)? => $e),+ }
    };
}
