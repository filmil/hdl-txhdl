// Probe 9. Operator latency, without a hand-placed stage boundary.
//
// A designer should not write `tick().await` to say where a register
// goes. That is the thing LHDL's eighth thesis asks the language to stop
// requiring. Instead the operators that have latency are async, and the
// stage boundaries are wherever the design awaits one.

/// Operators that may take a cycle. Every one of them is a pipeline
/// stage when it is awaited, which is why they live here and not beside
/// the combinational ones: the module a design imports from says whether
/// the operation can cost time.
pub mod pipeline {
    pub async fn mul(a: u32, b: u32) -> u64 {
        a as u64 * b as u64
    }

    pub async fn add(a: u64, b: u64) -> u64 {
        a.wrapping_add(b)
    }

    pub async fn div(a: u32, b: u32) -> u32 {
        if b == 0 { 0 } else { a / b }
    }
}

/// Operators that cannot. A plain `fn` has no `.await` to write, so
/// nothing here can span a cycle, and the import says so at the call
/// site.
pub mod funcs {
    pub fn low_half(x: u64) -> u32 {
        x as u32
    }

    pub fn high_half(x: u64) -> u32 {
        (x >> 32) as u32
    }

    pub fn select(c: bool, a: u32, b: u32) -> u32 {
        if c { a } else { b }
    }
}

use funcs::{low_half, select};
use pipeline::{add, mul};

/// Two stages, and neither boundary was placed by hand. `p` is live
/// across the await that `mul` introduces, so the async state machine
/// holds it, which is the work a `pipe` declaration used to ask for.
pub async fn mac(a: u32, b: u32, prev: u64) -> u64 {
    let p = mul(a, b).await;
    add(prev, p).await
}

/// Three operators, three awaits, and the depth follows from the
/// operators rather than from anything the designer counted.
pub async fn weighted(a: u32, b: u32, c: u32, d: u32) -> u64 {
    let x = mul(a, b).await;
    let y = mul(c, d).await;
    add(x, y).await
}

/// Why there is no `impl Add`. An operator has to return a value, and an
/// operation whose latency the mapping decides cannot. Providing `+`
/// would mean providing it only for the zero-latency case, and a design
/// that wrote `a + b` would have silently chosen a combinational adder.
/// Awaiting is not a claim that a cycle is spent; it is a refusal to
/// decide here.
pub async fn chain(a: u64, b: u64, c: u64) -> u64 {
    add(add(a, b).await, c).await
}

/// Timeless: everything it calls comes from `funcs`, so there is no
/// `.await` to write and no cycle count can be stated.
pub fn narrow(v: u64, take_low: bool) -> u32 {
    select(take_low, low_half(v), funcs::high_half(v))
}
