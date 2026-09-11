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
//! - `#[derive(Transaction)]`, `#[derive(Bus)]`, `interface!`,
//!   `when!` and `case!`, re-exported from `txhdl_macros`.
//!
//! This is a simulation-shaped prototype: values exist while it runs.
//! The lowering to hardware is the experiment the article states, and
//! this crate is what that experiment would be written against.

pub mod comp;
pub mod funcs;
pub mod pipeline;
pub mod types;

pub use txhdl_macros::{case, interface, when, Bus, Transaction};
