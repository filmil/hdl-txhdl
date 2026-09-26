// SPDX-License-Identifier: Apache-2.0
//! Assertions, assumptions and cover points: what a unit states about
//! itself, checked by the run as it goes and written into the netlist
//! for a simulator or a formal tool to check again (issue 502).
//!
//! Every other check in this crate compares: the netlist must do what
//! the Rust run did. That catches a lowering that is wrong, and says
//! nothing about a design whose Rust is wrong in the same way as its
//! netlist. A statement of what must hold is a check of the design
//! itself, and a formal tool can prove it for every input rather than
//! for the one run.
//!
//! Three statements, each a macro so that it reads as a statement in a
//! unit's `run`, each a condition and a message:
//!
//! - [`check!`](crate::check): the condition holds on every edge the
//!   statement is reached. The run panics with the message when it does
//!   not.
//! - [`assume!`](crate::assume): the condition is what the unit may
//!   take for granted of its inputs. A formal tool proves the checks
//!   under it and nothing else; the run, which cannot constrain its
//!   inputs, panics when its own inputs break it, since the proof would
//!   not cover that run.
//! - [`cover!`](crate::cover): the condition can happen. The run counts
//!   how often it did, and [`covered`] says; a formal tool looks for a
//!   trace that reaches it.
//!
//! Under `#[lower]` each is a statement of the process it is in, under
//! the conditions it is under, and is stated only while `rst` is low,
//! so that nothing is checked while the unit is held in reset, whether
//! the netlist added the reset or the unit declared it (issue 633). The
//! run has no reset to gate on and checks every edge the statement is
//! reached.
//! In the Verilog it is SystemVerilog's immediate `assert`, `assume` or
//! `cover` inside the clocked block, between `` `ifdef FORMAL `` and
//! `` `endif ``: a formal tool such as SymbiYosys defines `FORMAL`, and
//! a tool that reads plain Verilog, a synthesis tool, never sees them.
//! In the VHDL a check and an assumption are VHDL-2008's own `assert`,
//! which a simulator checks, and a cover point is a `report` of its
//! message when it is reached.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::comp::now;
use crate::types::Bit;

thread_local! {
    /// How often each cover point has been reached, by its message.
    static COVERED: RefCell<HashMap<&'static str, u64>> =
        RefCell::new(HashMap::new());
}

/// What [`check!`](crate::check) does in the run: panic, naming the
/// message and the time, when `cond` does not hold.
#[doc(hidden)]
pub fn check(cond: impl Into<Bit>, msg: &str) {
    if !cond.into().to_bool() {
        panic!("check failed at t={}: {msg}", now());
    }
}

/// What [`assume!`](crate::assume) does in the run: panic when the run's
/// own inputs break what the unit takes for granted of them.
#[doc(hidden)]
pub fn assume(cond: impl Into<Bit>, msg: &str) {
    if !cond.into().to_bool() {
        panic!("assumption broken at t={}: {msg}", now());
    }
}

/// What [`cover!`](crate::cover) does in the run: count a hit.
#[doc(hidden)]
pub fn cover(cond: impl Into<Bit>, msg: &'static str) {
    if cond.into().to_bool() {
        COVERED.with(|c| *c.borrow_mut().entry(msg).or_insert(0) += 1);
    }
}

/// How often the cover point with the message `msg` has been reached in
/// this run, on this thread.
pub fn covered(msg: &str) -> u64 {
    COVERED.with(|c| c.borrow().get(msg).copied().unwrap_or(0))
}

/// `check!(cond, "message")`: the condition holds on every edge at which
/// the statement is reached. The run panics with the message when it
/// does not; the netlist asserts it, for a simulator or a formal tool.
/// See [`formal`](crate::formal).
#[macro_export]
macro_rules! check {
    ($cond:expr, $msg:literal $(,)?) => {
        $crate::formal::check($cond, $msg)
    };
}

/// `assume!(cond, "message")`: what the unit takes for granted of its
/// inputs. A formal tool proves the checks under it; the run panics
/// when its own inputs break it. See [`formal`](crate::formal).
#[macro_export]
macro_rules! assume {
    ($cond:expr, $msg:literal $(,)?) => {
        $crate::formal::assume($cond, $msg)
    };
}

/// `cover!(cond, "message")`: the condition can happen. The run counts
/// it, which [`covered`](crate::formal::covered) reports; a formal tool
/// looks for a trace that reaches it. See [`formal`](crate::formal).
#[macro_export]
macro_rules! cover {
    ($cond:expr, $msg:literal $(,)?) => {
        $crate::formal::cover($cond, $msg)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_that_holds_says_nothing() {
        check(Bit::One, "holds");
        assume(true, "holds too");
    }

    #[test]
    #[should_panic(expected = "check failed at t=0: the count is a digit")]
    fn a_check_that_fails_stops_the_run_with_its_message() {
        check(false, "the count is a digit");
    }

    #[test]
    #[should_panic(expected = "assumption broken at t=0: the step is small")]
    fn an_assumption_the_run_breaks_stops_it() {
        assume(Bit::Zero, "the step is small");
    }

    #[test]
    fn a_cover_point_counts_the_steps_it_is_reached_on() {
        for k in 0..5 {
            cover(k % 2 == 0, "even");
        }
        assert_eq!(covered("even"), 3);
        assert_eq!(covered("never"), 0);
    }
}
