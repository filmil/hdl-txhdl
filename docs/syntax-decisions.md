<!-- SPDX-License-Identifier: Apache-2.0 -->
# Surface syntax for the merged language

Status: proposal, September 8, 2026.
Author: automated coding assistant, with human supervision.

`unification-analysis.md` settles what the merged language means.
This document settles how it is spelled.

The instruction is to bias the syntax toward Rust.
Rust is familiar, it is popular enough that engineers arrive already reading
it, and its constructs hold up under the coding patterns people now use.
The bias also earns its keep in a way that was not obvious in advance: it
fixes six defects in the two sources, listed in section 5.


## 1. The rule

Where LHDL and TxHDL disagree, and Rust has an answer, Rust wins.

Where Rust has no answer, because the construct is about hardware and not
about software, the better of the two sources wins, and section 4 says which
and why.

Rust's naming conventions apply throughout.
Types, traits, buses, transactions, and modules take `CamelCase`.
Values, functions, ports, and stages take `snake_case`.
Constants take `SCREAMING_SNAKE_CASE`.


## 2. The decision that does the most work

TxHDL declares state with `var` and combinational logic with `wire`.
LHDL argues, in thesis 8, that the designer should not make that distinction
at all.

Rust settles it with one keyword that neither source has.

```rust
let sum = a + b;            // a binding. Always equal to a + b. A wire.
let mut count: u32 = 0;     // a place. Holds a value between writes. A register.
```

`mut` states whether the value changes, which is a fact about the design.
It never states whether a flip-flop appears, which is a fact about the
implementation.
This satisfies LHDL thesis 8 and TxHDL's third principle, "variables become
registers automatically based on usage", with a single keyword that readers
already know.

Both `var` and `wire` are dropped.


## 3. Conflicts that Rust settles

| Concern | LHDL | TxHDL | Merged | Rust precedent |
|---|---|---|---|---|
| Assignment | `x := e` | `x = e` | `x = e` | `=` |
| Declaration | `var x: T;` | `var` / `wire` | `let` / `let mut` | `let mut` |
| Match arm | `case P: { .. }` | `P => { .. }` | `P => { .. }` | `match` arms |
| Match default | `default: { .. }` | `_ => { .. }` | `_ => { .. }` | `_` pattern |
| Enum variant | `case Data(u32);` | `Add = 0x00,` | `Data(u32),` and `Add = 0x00,` | enum with data and discriminants |
| Enum payload | `case Error(code: u8, ..)` | none | `Error { code: u8, fatal: bool },` | struct variant |
| Struct fields | `,` and `;`, both | `;` | `,` | struct definition |
| Struct literal | none | `{ .addr = 0x100 }` | `Req { addr: 0x100 }` | struct expression |
| Partial literal | implicit zero | implicit zero | `Req { addr: 0x100, ..default() }` | struct update syntax |
| Function | `func f() -> T` | `func f() -> T` | `fn f() -> T` | `fn` |
| Multi-cycle function | none | `proc` | `async fn` | `async fn` |
| Return | `return e;` | `return e;` | trailing `e`, or `return e;` | tail expression |
| Ternary | none | `c ? a : b` | `if c { a } else { b }` | `if` as expression |
| Cast | `u16(x)` | `zext(x)`, `i32(x)` | `x as u16` | `as` |
| Visibility | `pub` | none | `pub` | `pub` |
| Packages | `package a.b;` + `import` | none | `mod` + `use a::b;` | modules and paths |
| Path separator | `.` | `.` | `::` for items, `.` for fields | paths |
| Generics | `<type T>`, `<w: int>` | `<T, const N: u32>` | `<T, const N: u32>` | const generics |
| Generic defaults | none | `<T = u32, const N: u32 = 1024>` | same | default type parameters |
| Bit slice | `w[0..8]`, `w[12..=14]` | `w[7:0]` | `w[0..8]`, `w[12..=14]` | ranges |
| Bit order | `w[31 <- 0]`, `w[0 -> 31]` | none | dropped; use `.reverse_bits()` | no such operator |
| Array type | `u32[16]` | `u32[16]` | `[u32; 16]` | array type |
| Array literal | none | `[0; 16]` | `[0; 16]` | array repeat |
| Loop | `for (i in 0..10)` | `for (i in 0..10)` | `for i in 0..10` | no parentheses |
| Loop step | none | `for (i in 0..100 step 4)` | `for i in (0..100).step_by(4)` | `step_by` |
| Loop reverse | none | `for (i in 10..0 step -1)` | `for i in (0..10).rev()` | `rev` |
| Labels | none | `outer: for ..` / `break outer` | `'outer: for ..` / `break 'outer` | lifetime-style labels |
| Contract | `interface` + `implements` | structural | `trait` + `impl T for M` | traits |
| Role | `modport Master` | `.master` | `Bus::Master` | path to an associated item |
| Assertion | none | `assert c : "msg"` | `assert!(c, "msg")` | `assert!` |
| Parallel branches | `fork { branch A .. } join` | none | `let (a, b) = join!(f(), g());` | `join!` |
| Select | none | `select { .. }` | `select! { .. }` | `select!` |
| Await | none | `await port.request` | `port.request.await` | postfix `.await` |
| Send | none | `port.req <- { .. }` | `port.req.send(R { .. }).await` | no `<-` in Rust |
| Non-blocking send | none | `port.req <-? { .. }` | `port.req.try_send(R { .. })` | `try_` prefix |
| Callback | none | `then (resp) { .. }` | `.then(\|resp\| { .. })` | closures |
| Timeout | none | `timeout 100 else { .. }` | `.timeout(100).await` returning `Option` | `Option` and `match` |
| Tag application | `@Sync expr`, `@Sync { .. }` | none | `#[tag(Sync)]` on an item, block, or statement | attributes |
| Builtin constant fn | `$clog2(N)` | `$clog2(N)` | `clog2(N)` | `const fn` |
| Underscore literals | `0xDEAD_BEEF` | `0xDEAD_BEEF` | unchanged | already Rust |
| Sized literal | none | `8'd255` | `255u8` | typed literal suffix |

### On `.await`

Rust settled on postfix, and postfix chains where prefix does not.
Three of TxHDL's four await forms take it without argument:

```rust
let req = host.request.await;       // a transaction
cycles(BAUD_TICKS).await;           // a cycle count
let d = fetch_data(0x1000).await;   // an async fn
```

The fourth is `await condition`, and it has no Rust ancestor because Rust has
no cycle to wait for.
The proposal is that a `bool` is itself awaitable, and completes on the first
cycle where it holds:

```rust
(count == 0).await;
cts.await;
```

That keeps one rule instead of two.
It reads oddly the first time, which is the argument against it.
The alternative is a prefix `await cond;` statement alongside postfix
`.await` everywhere else, at the cost of two spellings for one idea.
This one is worth a human decision.


## 4. Where Rust has no answer

These constructs describe hardware, so Rust offers no precedent and the
better source wins.

| Concern | Decision | Why |
|---|---|---|
| Contract for wires | `bus` with `channel`, from TxHDL | LHDL's `interface` does two jobs at once, a wire bundle and an implementation socket, and that is why its grammar and its examples disagree. Splitting the jobs fixes both. |
| Contract for behavior | `trait`, replacing LHDL's `interface` in socket position | Late binding needs a name for a role an implementation fills. That is what a trait is. |
| Data compatibility | structural, from TxHDL | Two transactions with matching fields have matching layout, whatever they are called. |
| Contract conformance | declared with `impl`, from LHDL | Two modules can share a field set by accident. Intent should be written down. |
| Timeless behavior | `flow`, from LHDL's dataflow model | Cycle counts inside a `flow` are a compile error. The compiler schedules it. |
| Sequenced behavior | `seq`, from TxHDL's `sequence` | Cycle counts inside a `seq` are honored exactly as written. |
| Synchronization | `tag`, from LHDL, spelled as an attribute | `#[tag(MemFetch, handshake, capacity = 32)]` states in one place what LHDL's grammar and examples spell three different ways. |
| Clock and reset | `domain` in the configuration facet, from LHDL | A clock is a physical fact, and the design facet should not name one. |
| Late binding | `config` and `bind`, from LHDL | TxHDL has no mechanism at all. |
| Foreign code | `blackbox` and `foreign`, from LHDL | Same. |
| Pipeline stages | `stage`, from both | Both sources agree. |
| Cross-stage value | `pipe`, from LHDL | Live range analysis needs somewhere to say a value crosses a stage boundary. |
| Chaining | `.via(Grayscale).via(Blur)`, replacing LHDL's `\|>` | Rust chains with methods and has no `\|>`. The meaning is unchanged. |
| Contiguous memory | `Mem<u32, 4, 4>`, replacing LHDL's `a[i, j]` | `[[u32; 4]; 4]` stays jagged, and a single RAM block is asked for by name. A comma inside brackets is too quiet for a decision that changes the synthesized primitive. |


## 5. Six defects the Rust bias fixes

Each of these is listed in `unification-analysis.md` section 6 as a defect in
a source specification.
Adopting the Rust form removes it, rather than papering over it.

1. **TxHDL's undefined block-as-expression.**
   `await port.response timeout 100 else { ... }` requires the else block to
   end in a value, and blocks are never defined as expressions.
   `.timeout(100).await` returns an `Option`, and `match` supplies the
   default.
2. **TxHDL's C designated initializers.**
   `{ .addr = 0x1000 }` has no stated type and no stated rule for missing
   fields.
   `Req { addr: 0x1000, ..default() }` names the type and states the rule.
3. **LHDL's two assignment operators.**
   The operator table offers `=` and `:=`; the grammar offers only `:=`.
   Rust has one, so there is nothing left to disagree about.
4. **LHDL's `var` with an initializer that the grammar forbids.**
   `let mut counter: u32 = 0;` is one form, and it is the only form.
5. **LHDL's `func` versus `fn`.**
   The prose says one, the grammar says the other. Rust says `fn`.
6. **LHDL's tag spelled three ways.**
   `tag Sync;`, `tag @CreditDomain with CreditBased;`, and
   `tag @MemFetch with handshake, capacity=32;` disagree with the grammar
   and with each other.
   An attribute has one shape: `#[tag(MemFetch, handshake, capacity = 32)]`.

One defect is fixed by the LL(1) target rather than by Rust.
`proc_def` and `func_def` both begin with an optional `pub`, so a predictive
parser cannot choose between them on the `pub` token.
Merging `proc` and `func` into `fn` and `async fn` removes the conflict,
because the deciding token now follows `pub` directly.


## 6. Worked example

The same design three ways: a multiply accumulate unit that reads pairs of
words from memory, accumulates their products, and answers over a bus.

### As TxHDL states it today

TxHDL writes the protocol well and cannot state the tag, the contract, or the
binding.

```c
transaction MacRequest { addr: u32; count: u8; }
transaction MacResponse { acc: u64; }

bus MacBus {
    request: MacRequest;
    response: MacResponse;
}

module MacUnit {
    port host: MacBus.slave;
    port mem:  MemBus.master;

    var acc: u64 = 0;

    pipeline mac(a: u32, b: u32, prev: u64) -> u64 {
        stage mul { var p = zext<u64>(a) * zext<u64>(b); }
        stage add { return prev + p; }
    }

    sequence main {
        loop {
            var req = await host.request;
            acc = 0;
            for (i in 0..req.count) {
                mem.request <- { .addr = req.addr + i * 8, .write = false };
                var lo = await mem.response;
                mem.request <- { .addr = req.addr + i * 8 + 4, .write = false };
                var hi = await mem.response;
                acc = mac(lo.data[31:0], hi.data[31:0], acc);
            }
            host.response <- { .acc = acc };
        }
    }
}
```

### As LHDL states it today

LHDL states the contract, the tag, and the binding, and has no way to write
the loop.
The body below is where an LHDL author stops.

```
interface MacOp {
    addr  : uint<32>;
    count : uint<8>;
    acc   : uint<64>;

    modport Server { in addr; in count; out acc; }
}

tag @MemFetch with handshake, capacity=8;

pipeline Mac implements MacOp.Server {
    pipe p: u64;
    stage Mul { p := a * b; }
    stage Add { acc := p + acc; }
}

config Build {
    bind Top.mac_unit => Mac;
}
```

There is no LHDL construct that reads `count` words one after another and
waits for each response.
`fsm` would state it as hand written states, which is the form `await`
exists to replace.

### Merged

```rust
transaction MacRequest {
    addr: u32,
    count: u8,
}

transaction MacResponse {
    acc: u64,
}

bus MacBus {
    request: MacRequest,
    response: MacResponse,
}

/// Multiply accumulate over a memory-resident vector.
/// The host sends one request and gets one response.
pub trait MacOp {
    async fn run(&mut self, req: MacRequest) -> MacResponse;
}

/// Two stages, so the multiplier does not sit in the adder's path.
/// The compiler places the register between them.
pipeline mac(a: u32, b: u32, prev: u64) -> u64 {
    stage mul { let p = (a as u64) * (b as u64); }
    stage add { prev + p }
}

#[tag(MemFetch, handshake, capacity = 8)]
pub module MacUnit {
    port host: MacBus::Slave,
    port mem:  MemBus::Master,

    let mut acc: u64 = 0;

    seq main {
        loop {
            let req = host.request.await;
            acc = 0;

            for i in 0..req.count {
                let lo = mem.read(req.addr + i * 8).await;
                let hi = mem.read(req.addr + i * 8 + 4).await;
                acc = mac(lo.data[0..32], hi.data[0..32], acc);
            }

            assert!(req.count > 0, "empty accumulate has no defined result");
            host.response.send(MacResponse { acc }).await;
        }
    }
}

impl MacOp for MacUnit {}

config Build {
    bind top.mac_unit => MacUnit;

    domain MemFetch {
        clock: sys_clk_100mhz,
        reset: sys_rst_n,
    }
}
```

Nine differences from the TxHDL version are worth naming, because each one
is a decision from the tables above:

1. `var acc` becomes `let mut acc`, so the text says the value changes and
   not that a register exists.
2. `zext<u64>(a)` becomes `a as u64`.
3. `lo.data[31:0]` becomes `lo.data[0..32]`.
4. `await host.request` becomes `host.request.await`.
5. `mem.request <- { .addr = .. }` becomes a call that returns a value, so
   the request and its response are one line instead of two.
6. `{ .acc = acc }` becomes `MacResponse { acc }`, with the type named and
   the field shorthand doing the rest.
7. `sequence` becomes `seq`, and its cycle counts stay exactly as written.
8. The tag, the trait, and the configuration come from LHDL, and TxHDL had
   no way to write any of them.
9. `assert!` comes from TxHDL, and LHDL had no way to write it.


## 7. Keyword list

Grouped by what each keyword introduces.
Rust keywords keep their Rust meanings.

```
From Rust:
  mod use pub const static type struct enum trait impl fn async
  let mut match if else loop while for in break continue return
  as await move where true false _

Hardware items:
  module bus channel transaction protocol pipeline flow seq config

Hardware members and statements:
  port stage pipe instance connect bind tag domain
  blackbox foreign property

Macros:
  assert! assume! cover! select! join! concat! repeat!
```

`var`, `wire`, `func`, `proc`, `sequence`, `interface`, `modport`,
`implements`, `package`, `import`, `case`, `default`, `next`, `fork`,
`branch`, `join`, `state`, and `fsm` are all retired.
Their replacements are in the tables above.


## 8. What still needs a human decision

1. **`.await` on a condition.**
   Section 3 recommends `(count == 0).await` for one consistent rule, and
   notes that a prefix `await cond;` statement reads better in isolation.
2. **`flow` and `seq` as names.**
   Neither appears in either source.
   `flow` and `comb` are candidates for the first; `seq`, `proc`, and
   `task` are candidates for the second.
3. **Chaining.**
   Section 4 replaces `|>` with `.via(..)`.
   Keeping `|>` costs one non-Rust operator and reads well for long chains.
4. **`transaction` as a separate item.**
   It could be `#[transaction] struct MacRequest { .. }` instead, which is
   one fewer keyword at the cost of a longer declaration.
