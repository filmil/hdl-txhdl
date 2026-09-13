<!-- SPDX-License-Identifier: Apache-2.0 -->
# Embedding TxHDL in Rust

Status: design, September 11, 2026.
Author: automated coding assistant, with human supervision.

Every claim in this document about what Rust does was compiled.
The probes are in `//experiments/rust_embedding`, one per question, and
section 7 states what each one answered.
Where a probe failed, that failure is the answer and the probe is kept,
tagged `manual` so it stays out of `//...`.

The proposal is to stop defining a language and start defining a library.
TxHDL source becomes Rust source, `rustc` does the parsing, and every
construct Rust already provides is deleted rather than specified.


## 1. The rule

> Delete every construct Rust already provides.
> Rename every construct whose name collides with a Rust concept.
> What remains is the library.

The measure of success is how little remains.

`module` becomes `unit`, so that `mod` keeps its Rust meaning and nothing
has two meanings.


## 2. The vocabulary

Two modules hold everything the language still needs.

```rust
crate::types::Transaction   // data that moves between units
crate::types::Tag           // a synchronisation domain and its policy

crate::comp::Module         // a unit
crate::comp::Bus            // a bundle of channels
crate::comp::Signal         // one wire
crate::comp::Port           // a unit's attachment to a bus, in a role
crate::comp::Chan           // one channel of a bus
crate::comp::Role           // the type-level tag for a modport
```

Everything below is a plain Rust struct, trait or function.

| Was | Is now |
|---|---|
| `transaction T { .. }` | `struct T { .. }` plus `#[derive(Transaction)]` |
| `module M { .. }` | `struct M { .. }`, a unit, plus `impl Module for M` |
| `bus B { .. }` | `struct B { .. }` plus `#[derive(Bus)]` |
| `interface I { .. }` | `struct I { .. }` of `Signal` fields |
| `modport R { .. }` | `trait R`, implemented for the interface |
| `channel c: T` | a field of type `Chan<T>` |
| `port p` | a field of type `Port<B, R>` |
| `flow f(..)` | `fn f(..)` |
| `seq s { .. }` | `async fn` |
| `pipeline`, `stage`, `pipe` | `async fn` and `.await` (section 4) |
| `tag T` | `struct T` plus `impl Tag for T`, used as a type parameter |
| `config`, `bind` | a `type` alias (section 5) |
| `domain D` | a type parameter |
| `assert`, `assume`, `cover` | Rust's own macros (section 6) |

Seventeen constructs deleted. What is left is a library, not a keyword
list.


## 3. `fn` and `async fn` replace `flow` and `seq`

This is the part of the embedding that pays for itself.

The merged specification spends a section on the rule that a `flow` is
timeless and a `seq` counts cycles, and states that a cycle count inside a
`flow` is a compile error.

In Rust that rule needs no enforcement, because it cannot be broken.
A cycle count is spelled `.await`.
A plain `fn` is not `async`, so it has no `.await`, so it cannot state a
cycle count.
`rustc` rejects it already.

Two constructs deleted, and a semantic rule turned into a fact about the
type system.


## 4. A pipeline is an async fn

A pipeline and a sequence are one construct driven two ways.
A sequence runs one invocation to completion before starting the next.
A pipeline starts a new invocation every cycle, so several are in flight,
each at a different point in the body.
That difference belongs at the call site.

```rust
use crate::pipeline::{add, mul};

async fn mac(a: U<32>, b: U<32>, prev: U<64>) -> U<64> {
    let p = mul(a, b).await;   // the multiplier's latency
    add(prev, p).await         // and the adder's
}
```

Three constructs go, and one of them takes a compiler pass with it.

`pipe` declared a value that crosses a stage boundary so live range
analysis would hold it.
In an async fn a local live across an `.await` is held in the generated
state machine, because that is what async functions do.
`p` above crosses the await and needs no declaration.

Three things this does not settle:

* **Named stages are gone.** Boundaries are awaits, not named blocks.
* **The initiation interval moves to the call site.** From a keyword to an
  argument. A fair trade, not a free one.
* **Resource sharing needs proving.** A real pipeline shares one multiplier
  across every item in flight. Concurrent invocations are the same code at
  the same await index, so the lowering can share them, but that is a claim
  about the compiler rather than about Rust.

Stalling is the one that improves: it was a statement, and it is now an
await that has not completed.


## 5. A config is a struct

A config gathers every choice a build makes: which implementation fills
each socket, what each generic parameter is, and what the physical numbers
are.
A trait with associated types and associated constants holds all three, and
a struct implements it, which is the same shape as `Transaction`, `Bus` and
`Tag`.

```rust
pub trait Config {
    type Mac: MacOp;          // a socket
    type Clock: Tag;          // a domain
    const TAPS: usize;        // a generic parameter
    const CLK_HZ: u64;        // a physical number
}

pub struct Fpga;
impl Config for Fpga {
    type Mac = DspSliceMac;
    type Clock = Elastic;
    const TAPS: usize = 16;
    const CLK_HZ: u64 = 100_000_000;
}
```

The design is generic over the whole config rather than over one parameter
per choice, so adding a knob does not change every unit's signature.

The part worth noticing is what happens to the path.
`bind top.filter.mac => DspSliceMac` names a route through the instance
hierarchy, and that route goes stale whenever the hierarchy is rearranged.
A nested unit is generic over the same config, so the choice reaches it
with nobody naming a path:

```rust
pub struct Filter<C: Config> { pub mac: C::Mac, pub acc: u64 }
pub struct Top<C: Config>    { pub filter: Filter<C> }

pub type FpgaBuild = Top<Fpga>;
pub type AsicBuild = Top<Asic>;
```

Rearranging the hierarchy cannot invalidate a binding, because there is no
binding to invalidate.

`config`, `bind` and `domain` all disappear and what they expressed
survives. Probe 6 compiles it, including
`const TAP_BUFFER: [u64; FPGA_TAPS]`, which is a context that accepts
nothing but a true compile-time constant.

## 6. Non-synthesisable code is allowed

`assert!`, `println!` and the rest stay in the source.
They are Rust, so they cost the language nothing.

Synthesis skips them.
The rule that makes this safe is that a construct which is skipped may not
change the hardware: it may read state, and it may not drive anything.
An `assert!` that observes a signal is fine.
A skipped block that assigns to a signal would make the synthesised design
differ from the simulated one, and has to be an error rather than a
silent omission.

This deletes `assert`, `assume`, `cover` and `property` as language
constructs, and it also settles a question the merged specification had to
argue about at length. Section 3 of `unification-analysis.md` resolved
whether verification belongs in the language. Embedded in Rust the question
does not arise: Rust's constructs are already there.


## 7. What the probes established

Run them with `bazel build //experiments/rust_embedding/...`.
Two are tagged `manual` because they are expected to fail.

| Probe | Question | Answer |
|---|---|---|
| `probe_widths` | Does `U<{A + B}>` compile on stable? | **No.** `error: generic parameters may not be used in const operations`, on Rust 1.90.0 |
| `probe_widths_nightly` | Does it compile on nightly? | **Yes**, with `#![feature(generic_const_exprs)]`, and `mul(a, b)` resolves to `U<64>` at the call site |
| `probe_design` | Is `async fn` in a trait usable on stable? | **Yes**, with an `async_fn_in_trait` warning: auto trait bounds such as `Send` cannot be stated |
| `probe_design` | Can inputs and outputs be type parameters? | **Yes.** One struct implements `Module` twice with different shapes |
| `probe_design` | Interface as struct, modport as trait? | **Yes**, including a unit generic over the role rather than the bus |
| `probe_comp` | Do `Bus`, `Port`, `Signal`, `Chan` hold together? | **Yes**, and construction compiles |
| `probe_comp` | Is a `Tag` type parameter workable? | **Yes**, with the policy as associated constants readable in the body |
| `probe_attr_block` | Is `#[tag(..)]` on a block stable? | **No.** `error[E0658]: attributes on expressions are experimental` |
| `probe_processes` | Can `run` join two processes? | **Yes**, with processes taking `&self` and registers as cells |
| `probe_processes_mut` | Can they take `&mut self`? | **No.** `error[E0499]: cannot borrow *self as mutable more than once at a time` |
| `probe_config` | Is a config a struct with associated types and constants? | **Yes**, and its constants work in an array length, so they are genuinely compile-time |

Three of these change the design.

**The numeric library needs nightly**, or every width is declared by hand.
This is the largest single constraint in the proposal, and it is now
measured rather than guessed. The choice is `generic_const_exprs` on
nightly, or a `mul_32x32`-shaped function per width pair, which probe 1
shows compiles on stable.

**A tag cannot be an attribute on a block**, so it is a type parameter
instead. That was going to be the design anyway, and the probe turned a
guess into a reason.

**A unit's parallel processes cannot take `&mut self`.**
Hardware processes share state, and Rust permits one mutable borrow at a
time, so the two facts collide directly.
The repair is that a register is a `Cell` and a process takes `&self`.
`run` still takes `&mut self`; the processes it starts do not.
The repair also describes a register better than the version that failed:
a register is a thing several processes reach.


## 8. What Rust still cannot do

One wall is left, and it is the real one.

**Control flow on a signal.** A signal is a node in a graph, so it has no
value while the Rust program runs.

```rust
if cond { a } else { b }        // cond must be a bool. A signal is not.
```

`if` and `match` take a `bool` and a pattern and cannot be overloaded.
The ways out are `mux(cond, a, b)`, or a macro, or a proc macro that reads
the source and reinterprets `if` structurally.

**Bit slicing** is a smaller version of the same thing. `w[0..8]` cannot
work, because `Index` returns a reference to something that exists and a
slice of a signal is a new node. It becomes `w.slice::<0, 8>()`.


## 9. What it costs elsewhere

**Structural typing on data changes shape.**
The merged specification made transaction compatibility structural: two
transactions with matching fields are compatible whatever they are called.
Rust is nominal. A derive can recover it through an associated layout type,
but positionally, so the rule becomes "the same field types in the same
order" rather than "the same fields". That is a real change to a decision
already made, and it is not yet probed.

**`concat!` is taken** by `std`. The bit concatenation macro needs another
name.

**Thesis 2 is inverted.**
`filmil/theses.md` names embedding in an existing language as one direction
and then takes the other, on the grounds that a full toolchain is now
affordable. This proposal takes the direction the thesis declined. The
thesis is the rationale section of the merged specification, so it should be
rewritten rather than quietly contradicted.

The argument survives the change of direction, though. A toolchain is still
being built. It buys a `syn` front end rather than a lexer and a parser, and
the type system, the module system, the generics and the tooling come free.


## 10. The decision that governs the rest

Two ways to turn this Rust into hardware, and they are not compatible.

**Runtime elaboration.** The program runs, builds a netlist through side
effects, and emits Verilog. Full Rust is available for generating hardware
and no proc macros are needed. `if` on a signal stays impossible, so
section 8 stands.

**Proc macro elaboration.** `#[unit]` parses the item's source and emits
hardware from it. `if` and `match` on signals can be reinterpreted
structurally, which removes section 8, and errors arrive at compile time.
The cost is that the code looks like Rust and does not mean Rust.

The recommendation is **proc macro elaboration**, because section 8 is the
largest remaining cost of embedding and this is the only approach that
removes it.


## 11. Open questions

1. **Nightly or hand-declared widths.** Section 7. This one blocks the
   numeric library and nothing else can be designed around it.
2. **Does the layout derive give usable errors** when two transactions are
   not compatible? Not yet probed.
3. **Does `mux` read acceptably** in a page of real logic, or does
   section 8 force the proc macro? Write one page both ways.
4. **Is `Module` the right name for the trait**, given the item is called a
   unit? `crate::comp::Unit` would match the noun. The instruction said
   `Module`, and it is recorded here as asked.
