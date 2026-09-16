<!-- SPDX-License-Identifier: Apache-2.0 -->
# Instructions

Read `README.md` first.
It says what this repository is and how to build it.

The coding standard is the `ai-coding-sop` repository at
`https://github.com/filmil/ai-coding-sop`.
Its `AGENTS.md`, its `prose-readability` skill and its
`git-commit-rules` skill apply here in full.
In particular: no em-dashes or en-dashes anywhere, one sentence per line
in Markdown, conventional-commit titles, and the assistant note plus the
exact prompt appended to every commit message.

# What this repository holds

TxHDL is a hardware description language embedded in Rust.
The language is a library, `//lib`, and fourteen documents describe it,
all under `//docs`:

* `//docs:cover` names every document and says which to read for what.
  A new document is added there in the same change that creates it.
* `//docs:article` states the merge of the two languages this one came
  from.
* `//docs:embedding` states the embedding in Rust: the exposition of
  the language.
* `//docs:runtime` holds the runtime library verbatim, with the model
  of time its executor keeps stated in prose.
  Runtime internals belong here and nowhere else.
* `//docs:examples` holds every example verbatim, with the output the
  build produced by running it.
  The runtime appears there only through its public interface, which
  `//tools/api` extracts from the source at build time.
* `//docs:vreteno` is the first large design, the Vreteno RV32IM core
  under `//cpu/vreteno`, with its reference model, its lockstep test,
  its waveform, and the timer on its bus.
* `//docs:paper` is the expository paper: the system as it is and its
  results, with diagrams, for a reader meeting TxHDL for the first
  time; no history.
* `//docs:tutorial` is the tutorial, six steps on six examples for a
  reader new to the language; every listing is included from the
  examples and every printout is the build's.
* `//docs:zero` is the tutorial for an empty directory: four files
  that build a blinky with Bazel against the GitHub mirror, kept under
  `tutorial/blinky/`, which the tree builds as a check.
* `//docs:station` is the tutorial on the reservation station part:
  its rule read cycle by cycle on the example's run, the stations of
  two to ten inputs, the FIFO behind them, and the netlist.
* `//docs:axi` is the tutorial on the AXI link, the bus in
  `//lib/parts` under `bus::axi` and `bus::router`: the client verbs,
  accepting many transactions and answering them as the work
  finishes, the two trackers and their netlists, and the router, one
  host to several peripherals.
  Its three examples, `ex_axi`, `ex_axi_serve` and `ex_axi_router`,
  are documented there and not in `//docs:examples`, because a bus is
  a subsystem rather than one more construct.
* `//docs:gpu` is the minimal GPU under `//gpu`: a rasteriser that
  reads a display list and writes pixels into memory over the AXI
  link, the framebuffer behind the other end of it, the picture the
  build had it draw, and the netlists of both. It is the first client
  of the link that is hardware rather than a testbench.
* `//docs:cheatsheet` is the one-page cheat sheet, also as a PNG
  (`//docs:cheatsheet_png`); it shows `ex_cheat.rs`, its netlist and
  its waveform, all produced by the build.
* `//docs:stats` is the repository in numbers, lines per topic,
  measured once on September 13, 2026, and typed in; it is not kept up
  to date, and `tools/repostats.sh` measures again.
* `//docs:all` is every document above in that order, one PDF with a
  bookmark per document.

`docs/` holds the analysis behind every decision.
`experiments/rust_embedding/` holds the probes: one compiled question
per file, kept even when the answer was no.

# Standing rule: every concept lives in three places

When a new concept enters the language, it enters in three places in
the same change, or the change is not finished.

1. **The runtime.** Add it to `//lib`, in whichever of `types`, `comp`,
   `pipeline`, `funcs` or `macros` it belongs to.
   The library is the language; a concept that is only in a document
   is a proposal.
2. **A compiled example.** Add `lib/examples/ex_<concept>.rs` and its
   target in `lib/examples/BUILD.bazel`, and add the file to the
   `sources` filegroup there.
   The example is the test.
   A concept with no example is a concept nobody has checked, and
   `fragments/eng-standards.md` in the SOP requires that every piece
   of specification has a test that passes.
   A lowered example is also simulated against its own trace, as VHDL
   under nvc and as Verilog under Verilator: give `waveform()` in
   `docs/BUILD.bazel` a `lowered = (entity, unit)`, and the example
   calls `netlist::write_vhdl_from_env`; the tests
   `//docs:<name>_sim_<entity>_tb_test` and `//docs:<name>_vsim_test`
   must pass. A run of several units gives `lowered` a list of pairs
   and calls `netlist::write_netlists_from_env` with every unit; the
   tests are then `//docs:<name>_sim_<entity>_<entity>_tb_test` and
   `//docs:<name>_vsim_<entity>_test`.
3. **The documents.** Add a section for the example to
   `docs/examples_sections/`, with a paragraph on what it shows and a
   `\lstinputlisting` of the file; if it prints, add it to the
   `example_outputs` genrule in `docs/BUILD.bazel` and include the
   output after the listing.
   A new runtime file gets a section in `docs/runtime_sections/` and an
   entry in the `API` table of `docs/BUILD.bazel`.
   The documents include the files from the tree at build time, so they
   cannot drift from the code, and a listing must not be written by
   hand.
   The examples document may show the runtime only through its public
   interface; how a thing works inside is said in the runtime document.

The order is the order to work in.
Write the runtime, write the example against it, build both, then
include them.
Format with
`bazel run @rules_rust//:rustfmt --@rules_rust//:rustfmt.toml=//:rustfmt.toml -- //lib/...`
before including: the documents set source at 80 columns and never
break a line, so a longer line overflows its frame.
A listing that was typed into the article rather than included from a
file is a defect, because it is the one copy nothing checks.

# Claims about Rust are compiled, not argued

Any claim about what Rust accepts goes into
`experiments/rust_embedding/` as a probe before it goes into a document.
A probe whose answer is "no" is kept, tagged `manual` so it stays out of
`//...`, and its error message is the evidence.
The first draft of the embedding article argued from the language
reference and was wrong twice in eleven claims.

# Building

Everything is hermetic.
Nothing has to be installed beyond `bazelisk`.
The one external crate, `fst-writer`, comes through `crate_universe`
and is pinned by `Cargo.lock` and `cargo-bazel-lock.json`; change the
`crate.spec` in `MODULE.bazel` and run
`CARGO_BAZEL_REPIN=1 bazel build //lib:txhdl` to repin.
The two waveform converters are pinned in `multitool.lock.json`.

```sh
bazel build //...              # the library, every example, every document
bazel run //lib/examples:ex_config -- asic
bazel build //docs/...         # -> bazel-bin/docs/{cover,article,embedding,runtime,examples,txhdl}.pdf
bazel build //cpu/vreteno:vreteno_synth   # the core through Vivado, hermetic; manual
```

Vivado is hermetic through `rules_vivado`: the `vivado.install` tag in
`MODULE.bazel` names the AMD installer archive and the shared install
cache at `/data/cache/vivado-install`, and the key of that cache entry
is the URL and the selection with no `sha256`, exactly as the other
workspaces on this machine state them, so the installation is shared
rather than redone. A cold install elsewhere needs about 300 GB of
transient disk.

Before finishing a document change, run the readability pass from the
`prose-readability` skill, then render every page that holds a figure
and look at it.
A TikZ figure can be wrong with no warning and no error, and this
repository has had that happen twice.

# Verification

* `bazel build //...` is green.
* `bazel test //...` exits 0: the lowered units agree with their traces.
* Every PDF builds, every face is Type 1, every `\ref` and `\cite`
  resolves, and no listing line overflows its frame.
* Every page that holds a figure or a table was rendered and looked at:
  no float crosses into the other column or the margin, no float lies on
  another or on the text, and no label sits on a line or on another
  label. The build cannot see any of these.
* `CLAUDE.md` and `GEMINI.md` are symlinks to this file.
