// SPDX-License-Identifier: Apache-2.0
//! Operators that may take a cycle. Every one is `async`, and every one
//! is a pipeline stage the moment it is awaited. They live here and not
//! beside the combinational ones so that the module a design imports
//! from says whether an operation can cost time.
//!
//! There is no `impl Add`. An operator must return a value, and an
//! operation whose latency the mapping decides cannot. `.await` is not a
//! claim that a cycle is spent; it is a refusal to decide here.
//!
//! Widths are declared per function rather than computed, because
//! `U<{A + B}>` needs nightly Rust. The prototype pays that in
//! functions; a nightly build could pay it once.

use crate::comp::{process_in, tick, Clock, Rx, Tx};
use crate::types::Transaction;
use crate::types::U;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

// In the prototype each operator yields to the executor once, so it
// costs one cycle. A mapping would decide the real number.

pub async fn mul(a: U<32>, b: U<32>) -> U<64> {
    tick().await;
    U::new(a.raw() * b.raw())
}

pub async fn add<const N: usize>(a: U<N>, b: U<N>) -> U<N> {
    tick().await;
    a.wrapping_add(b)
}

pub async fn sub<const N: usize>(a: U<N>, b: U<N>) -> U<N> {
    tick().await;
    a.wrapping_sub(b)
}

pub async fn div(a: U<32>, b: U<32>) -> U<32> {
    tick().await;
    if b.raw() == 0 {
        U::new(0)
    } else {
        U::new(a.raw() / b.raw())
    }
}

/// Wait a stated number of cycles. This is the one place a design counts
/// them, and it is honoured exactly: a baud interval comes from the wire.
pub async fn cycles(n: usize) {
    for _ in 0..n {
        tick().await
    }
}

/// Drive a pipeline. At every edge of `C` at which an input is offered,
/// an invocation of `f` starts; every invocation in flight is polled
/// each step as a process of its own, so each advances one await per
/// cycle; and each result is sent as its invocation completes, oldest
/// first. Several invocations are in flight at once, each at a
/// different await, which is what a pipeline is; the awaits inside `f`
/// are its stage boundaries and nobody places them.
pub async fn drive<I, O, C, F, Fut>(f: F, input: Rx<I, C>, output: Tx<O, C>)
where
    I: Transaction,
    O: Transaction,
    C: Clock,
    F: Fn(I) -> Fut,
    Fut: Future<Output = O>,
{
    let mut flying: VecDeque<(Pin<Box<Fut>>, Waker, Option<O>)> =
        VecDeque::new();
    loop {
        C::edge().await;
        if let Some(x) = input.recv() {
            flying.push_back((Box::pin(f(x)), process_in::<C>(), None));
        }
        for (fut, w, done) in flying.iter_mut() {
            if done.is_none() {
                let mut cx = Context::from_waker(w);
                if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                    *done = Some(v);
                }
            }
        }
        if let Some((_, _, Some(_))) = flying.front() {
            if output.ready().to_bool() {
                let (_, _, v) = flying.pop_front().unwrap();
                output.send(v.unwrap());
            }
        }
    }
}
