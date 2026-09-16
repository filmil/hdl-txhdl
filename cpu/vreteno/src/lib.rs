// SPDX-License-Identifier: Apache-2.0
//! Vreteno: an RV32IM core in TxHDL, with the reference model it is
//! checked against and the programs it runs.
pub mod board;
pub mod core;
pub mod dmem;
pub mod isa;
pub mod model;
pub mod program;
pub mod run;
pub mod term;
pub mod timer;
pub mod uart;
