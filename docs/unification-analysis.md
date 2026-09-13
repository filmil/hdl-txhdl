<!-- SPDX-License-Identifier: Apache-2.0 -->
# Unifying LHDL and TxHDL

Status: analysis, September 8, 2026.
Author: automated coding assistant, with human supervision.

This document answers one question.
The repository states two hardware definition languages, and the goal is one
consistent specification.
What has to happen to get there?

The short answer is that the two languages do not compete.
LHDL says how a large system is assembled, configured, and scheduled.
TxHDL says how one module talks a multi-cycle protocol.
Each is almost silent where the other is detailed.
Four things genuinely conflict, and this document resolves all four.


## 1. What the repository holds today

Seven files state language content.
Their sizes say which ones matter:

| Path | Lines | Language | What it is |
|---|---:|---|---|
| `spec/language.md` | 2241 | TxHDL | The complete TxHDL specification, 20 numbered sections |
| `filmil/workspace/draft-spec.md` | 1218 | LHDL | The long LHDL draft, prose plus EBNF |
| `filmil/workspace/spec.md` | 486 | LHDL | A condensed LHDL specification with a consolidated grammar |
| `filmil/workspace/lhdl-proposal.md` | 178 | LHDL | The earliest LHDL sketch |
| `filmil/theses.md` | 94 | LHDL | The 12 design theses the language argues from |
| `proposal.md` | 27 | both | A prior agent's read of the repository and its next steps |
| `README.md` | 1 | none | One sentence naming the two authors |

`spec/language.md` has no Bazel target.
`filmil/workspace/BUILD.bazel` builds a PDF and an HTML page for each of the
four files under `filmil/workspace`, and the release workflow publishes
`bazel-bin/filmil` to hdlfactory.com.
The most complete specification in the repository is therefore the one that
never gets built or published.

The name appears four ways.
`draft-spec.md` writes FLHDL 88 times, `spec.md` writes it 9 times,
`lhdl-proposal.md` writes LHdl 6 times, and `spec/language.md` writes TxHDL
5 times.
The repository, the Bazel module, and the published path are all named
`txhdl`.


## 2. What each language is

### LHDL: dataflow, and time as an implementation detail

LHDL starts from 12 theses in `filmil/theses.md`.
Thesis 8 is the one that shapes the language: at the design level there
should be no notion of combinatorial versus sequential logic.
The designer writes dataflow and never counts cycles.
The compiler decides where registers go.

The mechanism is the **tag**.
A tag names a synchronization domain.
Operations sharing a tag are aligned by the compiler through longest path
analysis, which inserts bridge registers into the shorter paths.
Adding `with handshake` to a tag makes the domain elastic, and the compiler
replaces flip-flops with skid buffers and wires ready and valid across the
whole domain.
Adding `capacity=32` makes the compiler generate a scoreboard that tracks up
to 32 in flight transactions and asserts backpressure when full.

The organizing idea is the **three facets**.
The design facet states logic and contracts and is timeless.
The implementation facet maps logic to fabric and handles retiming.
The configuration facet binds implementations into sockets, sets generics,
maps tags to physical clocks, and links foreign code.
Changing the target fabric edits the configuration facet and leaves the
design facet untouched.

Reuse runs through **interfaces** with **modports**.
An interface declares fields; a modport names a role and gives each field a
direction for that role.
A module, a pipeline, or an FSM can implement an interface.

Verification is delegated.
LHDL adds no non-synthesizable subset.
Instead the configuration facet binds an interface to `foreign("cpp", ...)`
or `blackbox("verilog", ...)`, and the compiler generates a gRPC bridge and
bit exact software structs.

### TxHDL: transactions, and time written down

TxHDL starts from a different place.
Communication between modules happens through transactions over buses, never
raw signals.
Inside a module, one sequential flow is easy to reason about, while modules
run in parallel.
Variables become registers based on use.

The mechanism is **`await`**.
A sequence blocks until a transaction arrives, until a condition holds, or
for a stated number of cycles.
`select` waits on several events and takes the first.
`timeout N else { ... }` bounds the wait.
A protocol that takes 40 cycles reads as 40 lines of straight code rather
than as a state enumeration.

Communication runs through **buses** made of **channels**, each with implicit
valid and ready handshaking.
A channel marked `tagged by id` matches responses to requests out of order,
and `then (resp) { ... }` registers a callback that fires when the matching
response arrives.
`await_all` is the barrier.

Typing is **structural**.
Two transactions with the same fields are compatible whatever their names.
A transaction with extra fields can be sent where fewer are expected.

Verification is native.
`assert`, `assume`, `cover`, and `property` are in the language, and
properties use temporal operators (`always`, `eventually`, `next`).

### The asymmetry

Each language is thin exactly where the other is thick.

LHDL has no way to write a protocol body.
Its FSM is `state Idle { if (in_data > 0) { next => Processing; } }`, which
is the hand written state enumeration that `await` exists to remove.

TxHDL has no way to assemble a system.
Its top level offers `instance`, `connect`, and an address decoding bus.
There is no interface, no late binding, no configuration facet, no story for
wrapping legacy Verilog, and no story for swapping an implementation.

```
                  What LHDL states            What TxHDL states
                  ================            =================

  system          three facets                instance, connect
  assembly        config / bind / foreign      address-decode bus
                  domain -> clock mapping      domain(100MHz)
                  ####################         ..

  contracts       interface + modport          structural typing
                  implements                   on transaction fields
                  ###############              ########

  inter-module    tags, |>, fork/join          bus + channel
  communication   elastic buffers              valid/ready, tagged by
                  scoreboards                  then, await_all
                  ##############               ##############

  intra-module    pipeline + stage             sequence + await + select
  behaviour       state blocks                 pipeline + stage + stall
                  ....                         comb, wire, always
                  ..                           ######################

  types           u32, uint<W>, struct         u<N>, i<N>, bits<N>
                  enum with payloads           arrays, enums, structs
                  #####                        ###############

  verification    FFI to Go/Rust/C++           assert/assume/cover
                  gRPC co-simulation           property + temporal ops
                  #############                ##############

  grammar         full EBNF, LL(1) claim       none
                  ##########                   .

     ####  detailed        ..  named but not specified
```


## 3. The four real conflicts

Everything else is spelling, and section 4 of `syntax-decisions.md` settles
spelling.
These four need a decision about meaning.

### Conflict 1: does the designer count cycles?

LHDL says no.
TxHDL says yes, and `await cycles(BAUD_TICKS)` in the UART example counts
them explicitly.

**Resolution: both, with a boundary.**
The two answers apply to different things, and the conflict disappears once
that is stated.

A UART baud interval is a specification.
It comes from the protocol on the wire, and a compiler that retimed it would
break the design.
An adder's latency is not a specification.
It comes from the fabric, and a compiler that fixed it by hand would waste
the fabric.

So the merged language has two behavioral forms and a rule for choosing:

* **Timeless form**, from LHDL.
  Dataflow, tags, no cycle counts.
  The compiler schedules it.
  A cycle count written here is a compile error.
* **Sequenced form**, from TxHDL.
  `await` and explicit ordering.
  Cycle counts are honored exactly as written.

The boundary rule is that a sequence may call timeless code, and the call
absorbs whatever latency the compiler chose.
Timeless code may instantiate a sequenced block behind a contract, and sees
it as a fixed or elastic latency component.

### Conflict 2: nominal contracts or structural typing?

LHDL declares conformance with `implements`.
TxHDL infers it from matching fields.

**Resolution: structural for data, nominal for behavior.**
Each specification already uses its mechanism for what that mechanism is good
at.
Field compatibility between two transactions is a question about layout, and
layout is what structural typing decides well.
Whether a pipeline satisfies a bus contract is a question about intent, and
intent should be declared, because two modules can share a field set by
accident.

TxHDL section 14 keeps its meaning for transactions and structs.
LHDL's `implements` becomes a declared `impl`, and it is the only way to
claim a contract.

### Conflict 3: is verification in the language?

LHDL thesis 12 argues for interop with existing testing infrastructure
instead of a non-synthesizable subset.
TxHDL section 20 puts `assert`, `assume`, `cover`, and `property` in the
language.

**Resolution: both, split by what the construct talks to.**
The thesis argues against a simulation subset: file IO, string handling, and
stimulus generation.
It does not argue against formal properties.
`assert` and `property` are read by formal tools and by synthesis, not by a
simulator's runtime library, so they do not create the subset the thesis
objects to.

Stimulus, file IO, and scoreboarding move to the foreign function interface,
per LHDL.
Assertions, assumptions, cover points, and properties stay in the language,
per TxHDL.

### Conflict 4: where does a pipeline live, and does it have a signature?

LHDL makes `pipeline` a top level definition that may implement an
interface, and `pipe` declares a signal that persists across stages.
TxHDL makes `pipeline` a member of a module with a call signature,
`pipeline add(a: u32, b: u32) -> u32`, and `return` in the last stage.

**Resolution: TxHDL's shape, LHDL's placement.**
A pipeline is an item with a signature.
It may be declared at top level and may implement a contract, which is what
LHDL needs for late binding.
It may also be declared inside a module and called, which is what TxHDL needs
for the common case.
LHDL's `pipe` declaration stays, because live range analysis needs a place to
say that a value crosses stages.


## 4. What each specification contributes

The merge is not a compromise.
Most of both survives, because most of both is about something the other
never addressed.

**From LHDL:**

1. The three facets, as the top level organization of the specification.
2. Tags: synchronization domains, elastic flow, custom protocols, capacity
   and scoreboards.
3. Contracts with role views, from `interface` and `modport`.
4. Late binding: `config`, `bind`, `blackbox`, `foreign`.
5. Clock and reset abstracted into domains and mapped in configuration.
6. The foreign function interface and gRPC co-simulation, including
   automatic generation of bit exact software structs.
7. `|>` chaining and `fork` / `join` with automatic latency balancing.
8. Visibility with `pub`, and a package system.
9. The discipline of stating a grammar, and the LL(1) target.

**From TxHDL:**

1. Transactions, buses, and channels with implicit valid and ready.
2. `await`, `select`, `timeout`: the readable protocol body.
3. Tagged channels, `then` callbacks, and `await_all`.
4. Structural compatibility for transaction and struct types.
5. Generics in Rust form, `<T, const DEPTH: u32>`.
6. The concrete type system: `u<N>`, `i<N>`, `bits<N>`, arrays, enums with
   explicit encodings.
7. The bit manipulation set: slicing, concatenation, replication, `zext`,
   `sext`, reductions, `popcount`, `clz`, `ctz`, `ffs`.
8. Assertions, assumptions, cover points, and properties.
9. Module instantiation, instance arrays, and address decoding buses.
10. The lowering notes: sequences become state machines, `await` becomes a
    state transition, `then` becomes a pending table with a dispatcher.

**Dropped from both:**

* LHDL's `state` blocks for FSMs.
  A sequence with `await` states the same machine and reads better, and
  section 3 of this document already keeps both time models.
* TxHDL's Verilog style bit slice `word[7:0]` and width literal `8'd255`,
  replaced by Rust ranges and casts.
* LHDL's directional range operators `<-` and `->`.
  Rust has no such operator, and the same intent is written with a range and
  an explicit reversal.


## 5. The unified shape

The merged specification is organized by facet, and each facet states which
constructs belong to it.

```
  +-----------------------------------------------------------+
  |  DESIGN FACET                     (timeless, no cycles)   |
  |                                                            |
  |   trait          contract, with role views                |
  |   transaction    data that moves between modules          |
  |   bus            channels of transactions, valid/ready    |
  |   struct enum    data layout                              |
  |   flow           dataflow bodies, tagged, compiler-timed  |
  |   pipeline       staged bodies with a signature           |
  |   fn             pure combinational                       |
  +-----------------------------------------------------------+
                              |
                              | impl ... for ...
                              v
  +-----------------------------------------------------------+
  |  IMPLEMENTATION FACET             (cycles, when specified) |
  |                                                            |
  |   module         state, ports, instances                  |
  |   seq            sequenced body: await, select, then      |
  |   tag            synchronization domain and its policy    |
  |   protocol       custom forward/backward handshake        |
  |   assert assume cover property                            |
  +-----------------------------------------------------------+
                              |
                              | bind
                              v
  +-----------------------------------------------------------+
  |  CONFIGURATION FACET              (no logic at all)        |
  |                                                            |
  |   config         a named build                            |
  |   bind           socket <- implementation                 |
  |   domain         tag -> physical clock and reset          |
  |   blackbox       socket <- Verilog or VHDL                |
  |   foreign        socket <- Go, Rust, or C++ over gRPC     |
  +-----------------------------------------------------------+
```

The rule that makes this work is that a socket in the design facet names a
trait, never an implementation.
Both LHDL's `instance my_op: StreamingOp;` and TxHDL's
`instance mem: Memory<u32, 1024>;` are legal, and they differ in when the
implementation is chosen.
The first defers the choice to a `config`.
The second makes it at the instantiation site.


## 6. Defects in the sources

These are found by reading, not by a parser, because no parser exists.
Each has to be settled while merging, because a merged specification cannot
state two things at once.

### TxHDL, in `spec/language.md`

| Line | Defect |
|---:|---|
| 1 | The file begins with the literal text `:qODq` before `# TxHDL`. A stray vim keystroke was committed. |
| 660 | "TxHDL allows only one `sequence main` per module" contradicts `module Counter` at line 416, which declares `sequence main` and `sequence counter_tick` at line 451, and `module DualPortMemory` at line 2012, which declares `sequence handle_a` at line 2019 and `sequence handle_b` at line 2030. |
| 133, 141 | Inside one enum, `AND = 0x02` and `J = 0x02` share an encoding. |
| 135, 140 | Inside the same enum, `XOR = 0x04` and `BEQ = 0x04` share an encoding. |
| 1112 | `const SINE_TABLE: u8[256]` is initialized with 4 elements and a comment. |
| 740 | `await ... timeout 100 else { ... }` requires the else block to end in a value. Blocks as expressions are never defined. |
| 2043 | `port read1: bus { addr: u5; data: u32; }.slave;` uses an anonymous bus type inline. Section 4 never defines that form. |
| 1196 | `domain main_clock(100MHz)` uses a frequency literal. The literal section defines decimal, hex, binary, and sized literals, and no unit suffix. |
| whole file | No grammar. The quick reference lists keywords and operators, and never their syntax. |
| sections 3, 2 | `transaction` and `struct` are both product types, with no stated rule for choosing one. |
| section 14 | Subtype compatibility says a receiver ignores extra fields. The width of the physical channel for that case is not stated. |

### LHDL, in `filmil/workspace/`

The grammar and the examples disagree in sixteen places.
The first three rows below are enough on their own: no interface, modport,
or generic parameter list written in any of the four files parses against
the grammar in `spec.md`.

Paths below are relative to `filmil/workspace/`.
`spec.md` states its grammar twice, inline in sections 3 to 8 and again in
section 11, so each rule has two line numbers.
The first is cited.

| Where | Defect |
|---|---|
| `spec.md:85` and `:93` vs `spec.md:102` | `interface_item ::= port_decl ";"`, and `port_decl` requires a direction. Every interface writes undirected fields: `data_in  : T;` here, `in_data : T;` at `lhdl-proposal.md:39`, and `adr : uint<addr_w>;` in the Wishbone example at `draft-spec.md:134`. |
| `spec.md:91` vs `spec.md:105` | `modport_def` takes `port_decl ";"`, which requires a type. Every modport writes `modport Producer { out data_in;  in data_out; }`, with directions and no types. |
| `spec.md:96` vs `lhdl-proposal.md:38` | `generic_decl ::= "<" identifier ":" type ...`. `interface StreamingOp <type T>` writes the keyword first and does not parse. |
| `spec.md:223` vs `draft-spec.md:494` and `:515` | `tag_def` requires `":" type`. `tag Sync;` and `tag ElasticFlow with handshake;` omit it. |
| `spec.md:223` vs `draft-spec.md:542` | The grammar defines the tag name as a bare identifier. `tag @CreditDomain with CreditBased;` writes a leading `@`. |
| `spec.md:223` vs `draft-spec.md:560`, `lhdl-proposal.md:102` | The grammar reads `[ "with" identifier ] [ "capacity" "=" literal ]`, with no comma. `tag @MemFetch with handshake, capacity=32;` writes one. |
| `spec.md:266` vs `draft-spec.md:281` | `var_decl` has no initializer. `var counter: u32 := 0;` has one. |
| `spec.md:279` vs `draft-spec.md:1192` | `fork_stmt` ends `"join" [ tag_prefix ] [ "(" bind_list ")" ] ";"` in one file and `"join" ";"` in the other. Every example uses the first form. |
| `draft-spec.md:156` vs `spec.md:267` | The operator table says assignment is `=` or `:=`. `assign_stmt` has only `:=`. |
| `spec.md:269` vs `draft-spec.md:591` | State transition is `next "=>" identifier` in the grammar. The implicit tagging example writes a bare `=> State_Boot;`. |
| `draft-spec.md:405` vs `draft-spec.md:1131` | The prose says "Functions (fn)". The grammar says `func`. |
| `spec.md:121` vs `draft-spec.md:297` | `struct_def` separates fields with `,`, and `spec.md:132` follows it. `draft-spec.md:297` separates them with `;`. |
| `spec.md:162` vs `draft-spec.md:962` | `module_def` has no port list and no tag. `module AxiProcessor @AxiDomain ( bus : AxiStream<32> )` has both. |
| `spec.md:39` vs `draft-spec.md:983` and `:990` | `top_level_def` lists neither `domain` nor a bare `bind`. `domain AxiDomain { ... }` and `bind AxiProcessor => TopLevel;` both appear at top level. |
| `spec.md:183` vs `draft-spec.md:649` | `instance_decl` requires a parenthesized bind list. `instance my_op: StreamingOp;` has none. |
| `draft-spec.md:652` | `out_pixel := my_op(in_pixel);` calls an interface instance as a function. No rule covers that. |

The LL(1) claim is asserted and never checked.
One conflict is visible without tooling.
`top_level_def` includes both `proc_def` and `func_def`, and both begin with
an optional `pub`.
On the token `pub` a predictive parser cannot choose between them, so the
`pub` prefix has to be factored out before the claim holds.

### Repository level

* `spec/language.md` has no Bazel target, so the largest specification is
  never built.
  `filmil/workspace/BUILD.bazel` builds the four smaller files.
* The GitHub workflows triggered on `main` while the default branch was
  `dev` at the time. Both are now gone, and `main` is the default.
* The workflows live in `.github/workflows`, and the canonical remote is
  Forgejo, which reads `.forgejo/workflows`.
  Neither workflow runs.
  See issue https://git.hdlfactory.com/HDL/txhdl/issues/1.
* `.bazelversion` pins `9.0.1`.
  The coding SOP requires `9.2.0` or later, because earlier versions crash
  with a `NullPointerException` when fetching a repository named by a
  `file://` URL.


## 7. Decisions

**1. The name: TxHDL. Settled September 8, 2026.**
The repository, the Bazel module, and the published path are all `txhdl`,
and transactions are the distinguishing idea.
FLHDL, LHdl, and LHDL are retired.
They appear in this directory, and nowhere else, from here on.

**2. What happens to `filmil/workspace/`: deferred. September 8, 2026.**
The four files stay in the tree for now.
The options, for whenever this is picked up again, are that they merge into
the specification and then leave, or that they stay as source material.
`filmil/theses.md` is a separate case either way.
The theses are an argument, not a specification, and they belong in the
specification as a rationale section rather than being deleted.

One decision is still open here.
`syntax-decisions.md` section 8 lists three more, all about spelling.

**3. The behavioral keywords.**
Section 3 gives the merged language two behavioral forms.
`syntax-decisions.md` proposes `flow` for the timeless one and `seq` for the
sequenced one.
Neither name appears in either source, and both are open to a better one.


## 8. Suggested order of work

1. Settle the name.
2. Write `spec/language.md` section by section, in facet order, taking the
   type system and the bit manipulation set from TxHDL first, because
   nothing else can be written without them.
3. Write the EBNF as each section lands, rather than at the end, and check
   the LL(1) property with a generator instead of asserting it.
4. Fold the LHDL facets, tags, and configuration into the specification.
5. Fold the LHDL theses into a rationale section.
6. Delete `filmil/workspace/`, `proposal.md`, and `empty.md`, and move their
   Bazel targets onto the specification.
7. Build the documents as `document-build-plan.md` describes.

Step 3 matters most.
Both source specifications state a grammar or claim a property that their own
examples break, and the only reason that survived is that nothing checked it.
Every construct in the merged specification gets an example, and every
example gets parsed.
