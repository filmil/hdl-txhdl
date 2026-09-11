// Probe 9. Operator latency, without a hand-placed stage boundary.
//
// A designer should not write `tick().await` to say where a register
// goes. That is the thing LHDL's eighth thesis asks the language to stop
// requiring. Instead the operators that have latency are async, and the
// stage boundaries are wherever the design awaits one.

pub mod funcs {
    /// Operators whose latency is an implementation choice rather than
    /// the caller's. The caller awaits; how many cycles that costs is
    /// decided when the design is mapped.
    pub async fn mul(a: u32, b: u32) -> u64 {
        a as u64 * b as u64
    }

    pub async fn div(a: u32, b: u32) -> u32 {
        if b == 0 { 0 } else { a / b }
    }

    /// Combinational, so not async. Nothing here can span a cycle.
    pub fn low_half(x: u64) -> u32 {
        x as u32
    }

    pub fn select(c: bool, a: u32, b: u32) -> u32 {
        if c { a } else { b }
    }
}

use funcs::{low_half, mul, select};

/// Two stages, and neither boundary was placed by hand. `p` is live
/// across the await that `mul` introduces, so the async state machine
/// holds it, which is the work a `pipe` declaration used to ask for.
pub async fn mac(a: u32, b: u32, prev: u64) -> u64 {
    let p = mul(a, b).await;
    prev + p
}

/// Three operators, three awaits, and the depth follows from the
/// operators rather than from anything the designer counted.
pub async fn weighted(a: u32, b: u32, c: u32, d: u32) -> u64 {
    let x = mul(a, b).await;
    let y = mul(c, d).await;
    x + y
}

/// Timeless: combinational only, so there is no `.await` to write and no
/// cycle count can be stated.
pub fn narrow(v: u64, take_low: bool) -> u32 {
    select(take_low, low_half(v), low_half(v >> 32))
}
