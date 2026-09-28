// SPDX-License-Identifier: Apache-2.0
//! TxHDL, embedded in Rust: the runtime and the lowering's support.
//!
//! Every construct the language still needs, once everything Rust
//! already provides has been deleted. Nine modules and the macros:
//!
//! - [`types`]: `Bit`, `Logic`, `U<N>`, `I<N>`, `logic::Vec<N>`,
//!   the `Transaction` marker and the `Tag` trait.
//! - [`comp`]: units, wires and their ends, interfaces, clocks,
//!   configurations, `join2` and `parallel!`, `mux`, and the executor.
//! - [`comp::trace`]: names for signals, and a waveform writer, FST or
//!   VCD.
//! - [`pipeline`]: operators that may take a cycle. All `async`.
//! - [`funcs`]: operators that cannot. All plain `fn`.
//! - [`netlist`]: what a unit lowers to, its ports, registers and
//!   processes, and the Verilog and VHDL written from it; and the
//!   structural skeleton the same walk gives.
//! - [`foreign`]: a Verilog or VHDL module as a unit, run beside the
//!   Rust ones.
//! - [`map`]: an address map as a trait constant.
//! - [`regmap`]: what a map declared with `regmap!` stands for at run
//!   time, the table the tools read and the field constants.
//! - [`formal`]: `check!`, `assume!` and `cover!`, stated in a unit's
//!   `run`, checked by the run and written into the netlist.
//! - `#[lower]`, `with!`, `when!`, `case!`, `#[pipeline]`, `station!`,
//!   `regmap!`, `interface!` and the derives `Transaction`, `Bus`,
//!   `Value`, `Trace` and `Ports`, re-exported from `txhdl_macros`;
//!   and `select!`, a value chosen by pattern, defined here.
//!
//! A unit simulates as Rust, and `#[lower]` writes the same unit as
//! Verilog and VHDL, which the build checks against the run under nvc
//! and Verilator; the Vreteno core, written this way, runs on a board.

// Every public item carries its own documentation, and the build
// refuses one that does not. A type parameter is not covered by this
// lint, so a generic item says in prose what each of its parameters
// means and why any width that looks computable is stated instead.
#![deny(missing_docs)]

pub mod comp;
pub mod foreign;
pub mod formal;
pub mod funcs;
pub mod map;
pub mod netlist;
pub mod pipeline;
pub mod regmap;
mod reserved;
pub mod types;

pub use txhdl_macros::{
    case, interface, lower, pipeline, regmap, station, when, with, Bus, Ports,
    Trace, Transaction, Value,
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
