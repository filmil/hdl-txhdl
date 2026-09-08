# TxHDL

## The merged specification, in progress

This file is the specification that `spec/language.md` becomes.
Five sections are written.
The rest are listed with their titles, so the shape of the whole document
is visible before most of it exists.

The decisions behind every choice here live in `docs/`.
`docs/unification-analysis.md` states what LHDL and TxHDL each contribute
and resolves the four conflicts that needed a decision about meaning.
`docs/syntax-decisions.md` settles the spelling, construct by construct,
biased toward Rust.

Written here: sections 2, 3, 4, 8 and 14.

---

## Contents

**The design facet.** What the hardware does. No cycle counts.

1. Design philosophy and the three facets
2. **Types** (written)
3. **Transactions** (written)
4. **Buses** (written)
5. Traits: contracts and roles
6. Timeless behaviour: `flow` and `pipeline`

**The implementation facet.** How it maps onto cycles and fabric.

7. Modules
8. **Sequenced behaviour: `seq`, `await` and `select`** (written)
9. Tags and latency insensitivity
10. Expressions, operators and bit manipulation
11. Control flow
12. Functions and generics
13. Instantiation and composition

**The configuration facet.** What is bound to what. No logic.

14. **The configuration facet** (written)
15. Foreign code and co-simulation

**The rest.**

16. Assertions and properties
17. Memory and arrays
18. Grammar, as complete EBNF
19. Rationale: the twelve theses

---

## The running example

One example runs through the whole document.
A **finite impulse response filter**, or FIR filter, multiplies each of the
last N input samples by a fixed coefficient and adds the products together.
It is small enough to state in full and large enough to need every part of
the language.
It moves data over a bus, holds state, and pipelines arithmetic.
It talks a multi-cycle memory protocol.
Somebody has to choose which multiplier it uses on a given chip.

Each section develops the same filter rather than introducing a new toy.

---

## 2. Types

A type states how many bits a value occupies and how those bits are read.
Every type in TxHDL has a width the compiler knows at elaboration time.
There is no dynamic allocation and no pointer.

### 2.1 Scalars

Integers name their width in the type, following Rust.

```rust
let a: u8;        // 8 bits, unsigned
let b: u32;       // 32 bits, unsigned
let c: i16;       // 16 bits, signed, two's complement
let d: u<5>;      // 5 bits, unsigned
let e: i<12>;     // 12 bits, signed
let f: u<WIDTH>;  // width from a const generic parameter
```

`u8`, `u16`, `u32`, `u64` and `u128` are shorthand for `u<8>` and the rest.
The general form `u<N>` accepts any `N` the compiler can evaluate, so a
filter parameterised on its accumulator width writes `u<ACC_W>` and not one
type per width.

`bool` is one bit and supports the logical operators.
`u<1>` is one bit and supports arithmetic.
They convert with `as` in both directions and are otherwise distinct: a
condition takes a `bool`, and adding `bool` to `bool` is an error.

`bits<N>` is N bits with no arithmetic meaning.

```rust
let payload: bits<512>;   // opaque. Slice it, move it, do not add to it.
```

Use `bits<N>` where the bits are somebody else's format, such as a packet
body being forwarded.
The compiler rejects `+` on it, so a unit mistake stays a compile error
rather than becoming a subtly wrong adder.

### 2.2 Why there is no `wire` and no `var`

A declaration says whether a value changes, and never says whether a
flip-flop appears.

```rust
let scaled = sample * 2;      // a binding. Always equal to sample * 2.
let mut count: u8 = 0;        // a place. Holds its value between writes.
```

`let` introduces a name for an expression, so the two are equal at every
instant, and the compiler emits combinational logic.
`let mut` introduces storage that keeps its value until something writes
it, so the compiler emits a register.

This is Rust's `mut`, and it does the work that TxHDL's `wire` and `var`
split used to do.
It also satisfies the sixth of the twelve theses in section 19: the
designer states intent, and the combinational or sequential question is
answered during implementation.

Every `let mut` needs an initialiser, and that initialiser is its reset
value.
Section 9 states which reset.

### 2.3 Arrays

Arrays use Rust's syntax: the element type, a semicolon, the length.

```rust
let taps:    [i16; 16];             // 16 signed coefficients
let history: [i16; 16];             // the last 16 samples
let window:  [[i16; 16]; 4];        // 4 independent rows of 16
let zeros:   [u32; 8] = [0; 8];     // every element 0
let coeffs:  [i16; 4] = [1, 3, 3, 1];
```

An array of arrays is an array of independent arrays.
The compiler is free to synthesise each row separately, and a design that
reads two rows in one cycle gets what it asked for.

A single physical memory is asked for by name instead:

```rust
let frame: Mem<u32, 1024>;          // one RAM, one port unless stated
```

The distinction is deliberate and it is the one place TxHDL departs from
LHDL's spelling rather than its meaning.
LHDL wrote `a[i][j]` for independent rows and `a[i, j]` for one RAM.
A comma inside brackets is too quiet for a decision that changes which
primitive the synthesiser infers, so the merged language makes it a type.

### 2.4 Structs

A struct is a product type with named fields, separated by commas.

```rust
pub struct Sample {
    value:     i16,
    timestamp: u32,
}
```

Fields are laid out in declaration order, most significant first, and the
width of the struct is the sum of its field widths.
A struct with no fields occupies no bits.

Struct values are written with the type name and the fields:

```rust
let s = Sample { value: -12, timestamp: now };
let t = Sample { value: 0, ..s };       // every other field copied from s
```

Field shorthand applies when the variable already has the field's name, so
`Sample { value, timestamp }` means
`Sample { value: value, timestamp: timestamp }`.

### 2.5 Enums

An enum is a sum type.
A variant may hold nothing, a tuple, or named fields.

```rust
pub enum FilterMode {
    Bypass,
    Lowpass,
    Highpass,
}
```

The compiler chooses the encoding and the width unless told otherwise.
An explicit width and explicit discriminants are written like Rust:

```rust
pub enum Opcode: u<6> {
    Load  = 0x00,
    Store = 0x01,
    Run   = 0x02,
}
```

Two variants of one enum may not share a discriminant.
The compiler rejects it.

A variant may hold data:

```rust
pub enum Response {
    Idle,
    Data(i32),
    Error { code: u8, fatal: bool },
}
```

The width of such an enum is the width of the tag plus the width of its
widest variant.
Reading a payload requires `match`, so a payload can never be read from the
wrong variant:

```rust
match resp {
    Response::Data(v)               => acc = acc + v,
    Response::Error { code, .. }    => last_error = code,
    Response::Idle                  => {}
}
```

`match` is exhaustive.
A missing variant is a compile error, not a silent fallthrough.
Use `_` for a deliberate catch-all.

### 2.6 Type aliases

```rust
pub type Coefficient = i16;
pub type Accumulator = i<40>;
```

An alias is a second name for one type, not a new type.
`Coefficient` and `i16` are interchangeable everywhere.

### 2.7 Literals

```rust
let a = 42;                 // decimal
let b = 0xDEAD_BEEF;        // hex, underscores anywhere
let c = 0b1010_1100;        // binary
let d = 0o755;              // octal
let e = 255u8;              // typed
let f = -1i16;
```

An untyped literal takes the type demanded by its context.
Where no context demands one, it is `i32`.
A literal too wide for its type is a compile error, so `256u8` does not
compile and never silently becomes zero.

### 2.8 Casts

`as` converts between numeric types.

```rust
let wide  = narrow as u32;     // unsigned source: zero extended
let wide2 = signed as i32;     // signed source: sign extended
let small = wide as u8;        // truncates, keeping the low bits
```

The compiler reads the extension from the source type, so there is no
`zext` and no `sext`.
Truncation is always explicit: an assignment that would narrow without
`as` is an error.


## 3. Transactions

A transaction is the unit of data that moves between modules.

```rust
pub transaction SampleIn {
    value: i16,
}

pub transaction ResultOut {
    value: i32,
    saturated: bool,
}
```

A transaction declares fields exactly as a struct does.
It differs in what may be done with it.
A bus channel moves transactions, and it moves nothing else.

### 3.1 Structural compatibility

Two transactions are compatible when their fields match by name, type and
order.
Names of the transactions themselves are not compared.

```rust
pub transaction CpuWrite  { addr: u32, data: u32 }
pub transaction DmaWrite  { addr: u32, data: u32 }
```

A channel declared to take `CpuWrite` accepts a `DmaWrite`.
This is TxHDL's rule and it is kept, because compatibility of data is a
question about layout, and layout is what structural typing decides well.

Behavioural contracts work the other way.
Whether a module satisfies a role is declared with `impl`, never inferred,
because two modules can share a field set by accident.
Section 5 states that half.

### 3.2 Subsets

A transaction with a superset of the required fields may be sent where
fewer are expected, and the extra fields are dropped at the boundary.

```rust
pub transaction TaggedWrite { addr: u32, data: u32, id: u4 }

mem.write.send(tagged).await;   // legal where the channel takes CpuWrite
```

The channel's declared transaction fixes the width of the wires.
The sender drops them, so nothing downstream pays for the extra field.
Widening does not happen: a transaction missing a required field is an
error.

### 3.3 Generic transactions

```rust
pub transaction Packet<T, const CHECK_W: u32> {
    header:   u32,
    payload:  T,
    checksum: u<CHECK_W>,
}

let p: Packet<Sample, 16>;
```

### 3.4 Nesting

```rust
pub transaction Header { addr: u32, id: u8 }

pub transaction Request {
    header: Header,
    data:   u32,
}

let r = Request {
    header: Header { addr: 0x1000, id: 5 },
    data:   0,
};
```

Nested fields are read with `.`, as `r.header.addr`.


## 4. Buses

A bus is a named contract between modules.
It declares one or more channels, and each channel moves one transaction
type in one direction.

```rust
pub bus SampleStream {
    channel data: SampleIn,
}
```

Every channel has valid and ready handshaking, which the compiler inserts.
No design writes those signals.
A sender offers a transaction, a receiver accepts it, and the transfer
happens on the cycle where both hold.

### 4.1 Roles

A bus has two roles, and a port names the one it takes.

```rust
pub module FirFilter {
    port input:  SampleStream::Slave,    // receives on `data`
    port output: ResultStream::Master,   // sends on `result`
}
```

`Master` drives the channels declared with `channel`, and `Slave` receives
them.
A channel declared `channel back` reverses for both.

Where a bus needs more than two views, name them:

```rust
pub bus Wishbone<const ADDR_W: u32, const DATA_W: u32> {
    channel request: WbRequest<ADDR_W, DATA_W>,
    channel response: WbResponse<DATA_W>,

    role Initiator { out request, in  response }
    role Target    { in  request, out response }
}
```

`role` replaces LHDL's `modport`.
It states a direction per channel, and a port written `Wishbone::Initiator`
gets exactly those directions.
LHDL's `interface` did this job and the socket job at once, which is why its
grammar and its examples never agreed.
Here the two jobs are `bus` and `trait`.

### 4.2 Request and response

Most buses pair a request with a response.

```rust
pub transaction MemRead  { addr: u32 }
pub transaction MemData  { data: u32 }

pub bus MemBus {
    channel request:  MemRead,
    channel response: MemData,
}
```

A master sends on `request` and receives on `response`.
The two are independent channels: nothing forces one response per request,
and nothing forces them to alternate.
Section 8 shows the filter reading its samples over this bus.

### 4.3 Multi-channel buses

A bus may declare as many channels as the protocol has.
Each is handshaked separately, which is what makes an AXI-style bus
expressible without writing the handshakes:

```rust
pub bus Axi4 {
    channel aw: AxiAddr,
    channel w:  AxiData,
    channel b:  AxiResponse,
    channel ar: AxiAddr,
    channel r:  AxiReadData,
}
```

### 4.4 Tagged channels

A channel marked `tagged by` matches responses to requests by a field
instead of by order.

```rust
pub transaction TaggedRead   { id: u4, addr: u32 }
pub transaction TaggedResult { id: u4, data: u32 }

pub bus OooMemBus {
    channel request:  TaggedRead,
    channel response: TaggedResult tagged by id,
}
```

Responses may then arrive in any order.
Section 8 shows the callback form that reads them.

### 4.5 Sideband

A signal that has no handshake is declared `signal`.

```rust
pub bus MemBusWithIrq {
    channel request:  MemRead,
    channel response: MemData,

    signal irq: bool,
}
```

A `signal` is a wire.
It is readable at every instant by both ends, it has no valid and no ready,
and it may not be awaited.
Use it for a level that is always meaningful, such as an interrupt line or
an error flag.

### 4.6 Anonymous buses

A bus used in one place may be declared inline:

```rust
port debug: bus { channel probe: u32 } ::Slave,
```

The compiler generates the same handshaking.
Two inline buses with matching channels are compatible, by the rule in
section 3.1.



## 8. Sequenced behaviour: `seq`, `await` and `select`

A `seq` block states what a module does, in order, over as many cycles as
it takes.
It is the half of the language where the designer counts cycles, and every
count written here is honoured exactly.

The other half is `flow`, in section 6, where no cycle count may be
written and the compiler decides the schedule.
Section 1 gives the rule for choosing.
The short form is this.
A number that comes from a protocol on a wire is a specification, and it
belongs in a `seq`.
A number that comes from how fast the fabric happens to be is not a
specification, and it belongs nowhere.

### 8.1 A sequence

```rust
pub module FirFilter {
    port input:  SampleStream::Slave,
    port output: ResultStream::Master,
    port mem:    MemBus::Master,

    const TAPS: u32 = 16;

    let mut history: [i16; 16] = [0; 16];
    let mut acc: i<40> = 0;

    seq main {
        loop {
            let s = input.data.await;

            history[0] = s.value;
            acc = 0;

            for i in 0..TAPS {
                let tap = mem.request.send(MemRead { addr: TAP_BASE + i * 4 }).await;
                let coeff = mem.response.await;
                acc = acc + (history[i] as i<40>) * (coeff.data as i<40>);
            }

            output.result.send(ResultOut {
                value: (acc >> 16) as i32,
                saturated: false,
            }).await;
        }
    }
}
```

Read it top to bottom.
The filter waits for a sample, shifts it in, reads sixteen coefficients one
after another, and answers.
Each `await` is a point where the sequence stops until something happens,
and the compiler turns the whole block into a state machine with one state
per such point.

Nothing in that block says which cycle anything happens on, and nothing
needs to.
The memory answers when it answers.

### 8.2 The four things you can await

**A channel.** The sequence stops until a transaction arrives.

```rust
let s = input.data.await;
```

**A condition.** The sequence stops until the expression holds.

```rust
(fifo_level > 0).await;
calibration_done.await;
```

A `bool` is awaitable, and completes on the first cycle where it is true.
This keeps one rule rather than two.
It reads oddly the first time and it composes: anything that yields a
`bool` may be awaited without a second keyword.

**Cycles.** The sequence stops for a stated number of them.

```rust
cycle().await;              // exactly one
cycles(BAUD_TICKS).await;   // exactly BAUD_TICKS
cycles(delay).await;        // a value computed at run time
```

This is the form that has to be honoured literally.
A UART's bit interval comes from the wire, and a compiler that retimed it
would break the link.

**Another sequence.** An `async fn` may await, so calling one may take
cycles.

```rust
async fn read_word(addr: u32) -> u32 {
    mem.request.send(MemRead { addr }).await;
    mem.response.await.data
}

seq main {
    loop {
        let a = read_word(0x1000).await;
        let b = read_word(0x2000).await;
        output.result.send(ResultOut { value: (a + b) as i32, saturated: false }).await;
    }
}
```

A plain `fn` is combinational and may not await.
The compiler enforces it, so a function used inside a `flow` cannot quietly
acquire a latency.

### 8.3 Sending

```rust
output.result.send(r).await;         // blocks until the receiver accepts

if output.result.try_send(r) {       // returns bool, never blocks
    sent = sent + 1;
}
```

`send` offers the transaction and waits for the handshake.
`try_send` offers it for one cycle and reports whether it was taken.

Transactions are written as struct literals, so the type is named and the
missing fields are stated rather than assumed:

```rust
output.result.send(ResultOut { value: v, saturated: false }).await;
output.result.send(ResultOut { value: v, ..default() }).await;
```

### 8.4 Timeouts

`timeout` bounds a wait and yields an `Option`.

```rust
let coeff = match mem.response.timeout(100).await {
    Some(c) => c,
    None    => {
        timeouts = timeouts + 1;
        MemData { data: 0 }
    }
};
```

The wait either produced a value or it did not, and `match` makes the
design state what happens in each case.
TxHDL used to write `timeout 100 else { ... }` with a block that had to end
in a value, and blocks as expressions were never defined.
`Option` removes the special case.

### 8.5 Waiting on several things

`select!` waits on several events and takes the first to happen.

```rust
seq main {
    loop {
        select! {
            s = input.data.await        => { push_sample(s); }
            c = config.request.await    => { reconfigure(c); }
            _ = cycles(TIMEOUT).await   => { report_idle(); }
        }
    }
}
```

Exactly one arm runs.
When two are ready on the same cycle, the choice is unspecified, and a
design that needs an order says so:

```rust
select! { priority
    c = config.request.await => { reconfigure(c); }
    s = input.data.await     => { push_sample(s); }
}
```

An arm may be guarded, and a guarded arm whose guard is false is not
considered:

```rust
select! {
    s = input.data.await if running => { push_sample(s); }
    _ = start.await if !running     => { running = true; }
}
```

### 8.6 Out-of-order responses

A channel declared `tagged by` in section 4.4 matches a response to its
request by a field.
`then` registers what to do when that response arrives, and does not block.

```rust
seq main {
    let mut coeffs: [u32; 16] = [0; 16];
    let mut pending: u<5> = 0;

    for i in 0..16 {
        mem.request
            .send(TaggedRead { id: i as u4, addr: TAP_BASE + i * 4 })
            .then(|r| {
                coeffs[r.id as u32] = r.data;
                pending = pending - 1;
            });
        pending = pending + 1;
    }

    (pending == 0).await;
    // every coefficient has landed
}
```

Sixteen reads are outstanding at once, and they may return in any order.
The closure runs when the matching response arrives.

`join_all()` is the same barrier without the counter:

```rust
join_all().await;
```

A named handler replaces the closure where the same work is done in several
places:

```rust
handler store_coeff(r: TaggedResult) {
    coeffs[r.id as u32] = r.data;
}

mem.request.send(TaggedRead { id, addr }).then(store_coeff);
```

### 8.7 More than one sequence

A module may declare as many `seq` blocks as it needs, and they run
concurrently.

```rust
pub module DualPortMemory<const DEPTH: u32> {
    port a: MemBus::Slave,
    port b: MemBus::Slave,

    let mut cells: Mem<u32, DEPTH>;

    seq port_a {
        loop {
            let r = a.request.await;
            a.response.send(MemData { data: cells[r.addr] }).await;
        }
    }

    seq port_b {
        loop {
            let r = b.request.await;
            b.response.send(MemData { data: cells[r.addr] }).await;
        }
    }
}
```

TxHDL used to say a module held one sequence, while three of its own
examples declared two.
The rule here is that concurrency between sequences works exactly as
concurrency between modules does, one level down.

Two sequences that write the same `let mut` in the same cycle are a compile
error.
The compiler finds it by checking writers per cycle, so it is caught at
build time and not by a waveform.

### 8.8 Reset

Every `seq` restarts from its first statement on reset, and every `let mut`
returns to its initialiser.
There is no separate reset block to keep in step with the declarations.

Which reset, and whether it is synchronous, comes from the tag the module
runs under, and the tag is mapped to a physical reset in the configuration
facet.
Section 14.3 shows that mapping.

### 8.9 How it is built

A `seq` becomes a state machine.
Each `await` is a state boundary: the machine holds its state until the
awaited thing happens, then advances.

```
    let s = input.data.await;        S0: if (input.data.valid) -> S1
    acc = 0;                         S1: acc <= 0;             -> S2
    for i in 0..TAPS { ... }         S2..S4: the loop body
    output.result.send(..).await;    S5: if (output.result.ready) -> S0
```

A `then` callback becomes an entry in a pending table plus a dispatcher
that matches the tag field and runs the closure.
`join_all` becomes a comparison against the table's occupancy.

A loop whose bound is known at elaboration time may be unrolled, and one
whose bound is not stays a counter.
Section 11 states which.


## 14. The configuration facet

Nothing in this section describes logic.
The configuration facet says what is bound to what, which clock a tag runs
on, and where a component's implementation comes from.
It is the only facet that names a frequency, a file, or a foreign language.

Changing the target fabric edits this facet and leaves the design facet
untouched.
That is the point of having it: a design that has been verified stays
verified when the multiplier underneath it changes.

### 14.1 Sockets

A module may instantiate a trait rather than a concrete implementation.
The result is a socket: a hole in the design with a declared shape and no
occupant.

```rust
pub trait MultiplyAccumulate {
    flow run(a: i16, b: i16, prev: i<40>) -> i<40>;
}

pub module FirFilter {
    port input:  SampleStream::Slave,
    port output: ResultStream::Master,

    instance mac: MultiplyAccumulate;      // a socket, not a component

    // ... uses mac.run(...) as though it were there
}
```

The design compiles, simulates against a behavioural model, and cannot be
synthesised until something fills the socket.

### 14.2 `config` and `bind`

A `config` names a build.

```rust
config Fpga {
    bind top.filter.mac => DspSliceMac;
}

config Asic {
    bind top.filter.mac => WallaceTreeMac;
}
```

`bind` names the socket by its path through the instance hierarchy, then
the implementation that fills it.
The implementation must `impl` the socket's trait, and the compiler checks
that before anything else.

Two configurations of one design differ only here.
`FirFilter` is not edited, recompiled by hand, or forked.

Generic parameters are set in the same place:

```rust
config Fpga {
    bind top.filter.mac => DspSliceMac;
    set  top.filter.TAPS = 16;
    set  top.filter.ACC_W = 40;
}
```

### 14.3 Domains: where clocks come from

A tag names a synchronisation domain in the design facet, and says nothing
about frequency.
A `domain` block maps that tag onto a physical clock and reset.

```rust
config Fpga {
    domain SampleClock {
        clock: sys_clk,
        reset: sys_rst_n,
        reset_kind: async_assert_sync_release,
        frequency: 100 MHz,
    }

    domain MemClock {
        clock: ddr_clk,
        reset: sys_rst_n,
        frequency: 400 MHz,
    }
}
```

`frequency` is a number and a unit, and the units are `Hz`, `kHz`, `MHz`
and `GHz`.
The compiler uses it to check that a `flow` fits in a cycle, and to size the
elastic buffers a tag needs.
It does not generate a clock.

A design whose data crosses from one domain to another gets a clock domain
crossing inserted at the boundary.
The compiler knows a crossing happened because the two tags differ, so a
crossing cannot be forgotten.
It cannot insert one across a `signal`, which is why a `signal` may not
cross a domain boundary and the compiler rejects it.

### 14.4 Filling a socket with existing Verilog

```rust
config Fpga {
    bind top.filter.mac => blackbox("verilog", "dsp48_mac.v") {
        module: "dsp48_mac",
        ports: {
            a    => "A",
            b    => "B",
            prev => "PCIN",
            run  => "P",
        },
        latency: 3,
    };
}
```

The compiler stops elaborating at the boundary and checks widths across it,
so a `i16` bound to a `[7:0]` port fails the build.
`latency` states how many cycles the block takes.
The surrounding tag uses that number to balance the paths around it, which
is what lets a fixed latency Verilog block sit inside an elastic domain.

### 14.5 Filling a socket with software

A socket may be filled by a program instead of by hardware.

```rust
config Simulation {
    bind top.filter.mac => foreign("rust", "mac_model.rs");
    bind top.memory     => foreign("cpp",  "ddr_model.cpp");
}
```

The compiler generates a bridge over gRPC on a Unix domain socket, and
bit-exact structs in the target language, so the model reads named fields
and never shifts bits by hand.
The bridge steps the simulation one cycle at a time, so the model sees
every cycle the hardware sees.

This is how TxHDL avoids a non-synthesisable subset.
File reading, stimulus generation and scoreboarding happen in Go, Rust or
C++, where those things already work.
Section 16 keeps assertions and properties in the language, because a
formal tool reads those and a simulator's runtime does not.

Early in a project the model is the only implementation.
Later the same socket is bound to real hardware, and the parent module does
not change:

```rust
config Asic {
    bind top.filter.mac => WallaceTreeMac;
    bind top.memory     => Ddr4Controller;
}
```

### 14.6 What may not appear here

The configuration facet holds no expressions, no `if`, no `seq`, and no
assignment to a signal.
A `config` that computes something is stating design intent in the wrong
place, and the compiler rejects it.
The rule keeps the answer to "what does this build do differently" short
enough to read.
