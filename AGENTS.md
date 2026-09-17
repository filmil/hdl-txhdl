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
The language is a library, `//lib`, and twenty-one documents describe it,
all under `//docs`:

* `//docs:cover` names every document and says which to read for what.
  A new document is added there in the same change that creates it.
* `//docs:showcase` is at most five pages on the whole of it: the
  rule, the parts, Vreteno, Razboj, how it is all checked, and what is
  not done. It is the one document meant to be read first and alone,
  so it stays within five pages and every number in it comes from a
  count rather than from memory. Its tables claim to be exhaustive, so a
  new component goes in them in the same change; the standing rule
  below says what counts as one and how to check the numbers.
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
* `//docs:vreteno` is the first large design, the Vreteno RV32IMC core
  under `//cpu/vreteno`, with its reference model, its lockstep test,
  its waveform, the timer on its bus, the platform-level interrupt
  controller in `//lib/parts` under `plic` with its example `ex_plic`,
  and the board: one lowered module with the interrupt controller and
  the DDR3 memory of `//ddr3` behind it, UberDDR3's controller a
  foreign module inside, simulated whole under Vivado's simulator by
  the manual `//cpu/vreteno/board/sim:board_test`.
  `ex_plic` is documented there and not in `//docs:examples`.
* `//docs:tapeout` is the same core after the FPGA: mapped onto the
  open Nangate45 standard cell library with Yosys and floorplanned,
  placed, routed and timed with OpenROAD, both fetched by checksum as
  Debian packages and unpacked by the build. The mapping is a normal
  target; the layout is `//cpu/vreteno/asic:vreteno_pnr`, which is
  manual because it is twenty minutes of work, and what it writes into
  `docs/asic.tsv` and `docs/asic_map.tsv` is committed, so
  `//tools/denmap` draws the figure and the numbers at build time.
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
  `//lib/parts` under `bus::axi`, `bus::router`, `bus::axi_lite` and
  `bus::wb`: the client verbs, accepting many transactions and
  answering them as the work finishes, the two trackers and their
  netlists, the router, one host to several peripherals, AXI-Lite with
  the bridge from an AXI4 link to several AXI-Lite peripherals, the
  bridge that reaches a Wishbone peripheral, and the simplest
  peripheral, one register written as hardware.
  Its six examples, `ex_axi`, `ex_axi_serve`, `ex_axi_reg`,
  `ex_axi_router`, `ex_axi_lite` and `ex_axi_wb`, are documented there
  and not in
  `//docs:examples`,
  because a bus is a subsystem rather than one more construct.
* `//docs:razboj` is Razboj, the minimal GPU under `//gpu/razboj`: a
  rasteriser that reads a display list and writes pixels into memory over the AXI
  link, the framebuffer behind the other end of it, the picture the
  build had it draw, and the netlists of both. It is the first client
  of the link that is hardware rather than a testbench.
* `//docs:noc` is the network on chip in `//lib/parts` under
  `bus::noc`: a node with a link to each of its four neighbours and an
  exit, two virtual channels, dimension-order routing, and the bridge
  that puts an AXI link across it. Its tests drive four nodes, both
  bridges and a memory, and `//soc` puts Vreteno, Razboj, a memory and a
  serial port on the four corners of one lattice and runs them
  together.
* `//docs:eth` is the Ethernet part in `//lib/parts` under `eth`: the
  MAC's transmitter and receiver on GMII, each on one clock and each
  storing whole frames, the CRC-32, the AXI-Lite peripheral, the
  example `ex_eth` on a looped wire, and `//eth`, the hand-written
  RGMII wrapper, clock crossing and echo design for the AX7A200B's
  JL2121 PHY, which `//eth:echo_synth` and `//eth:echo_pnr` put
  through Vivado to a bitstream.
  `ex_eth` is documented there and not in `//docs:examples`.
* `//docs:hdmi` is the HDMI part in `//lib/parts` under `hdmi`: the
  video peripheral for an encoder chip, its raster and its framebuffer
  a host paints over AXI-Lite, the I2C master that configures the chip,
  the example `ex_hdmi`, and `//hdmi`, the hand-written demonstration
  for the AX7A200B's SiI9134, which `//hdmi:demo_synth` and
  `//hdmi:demo_pnr` put through Vivado to a bitstream.
  `ex_hdmi` is documented there and not in `//docs:examples`.
* `//docs:datasheets` is a datasheet per component: every unit under
  `#[lower]` outside the examples, and every family a macro writes.
  The prose of each sheet is in `docs/datasheets/<Key>.tex`; its
  tables of ports, state and children are generated at build time by
  `//tools/datasheet` from the component's own lowering.
  See the standing rule below.
* `//docs:cheatsheet` is the one-page cheat sheet, also as a PNG
  (`//docs:cheatsheet_png`); it shows `ex_cheat.rs`, its netlist and
  its waveform, all produced by the build.
* `//docs:stats` is the repository in numbers, lines per topic,
  measured once on September 13, 2026, and typed in; it is not kept up
  to date, and `tools/repostats.sh` measures again.
* `//docs:dynamics` is how the work went rather than what it is: the
  project's timeline measured from its own history, with a Gantt
  chart of the bands of work and what each needed before it could
  begin.
  `tools/timeline` extracts the data into `docs/timeline.tsv` by
  hand, because a Bazel action has no repository to read, and
  `//tools/gantt` draws it at build time.
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

# Standing rule: a component enters the showcase in the same change

`//docs:showcase` is the one document meant to be read first and
alone, and its two tables claim to be exhaustive: Table I says it
lists *every* unit the build lowers, and Table II *every* system it
assembles.
A change that adds one and does not add it there makes the document
lie, and a reader has no way to tell which of the two is out of date.

So a change that adds any of the following updates `docs/showcase.tex`
in the same commit:

* a unit others can use: a part, a peripheral, a bridge;
* a system the build assembles from units;
* a toolchain the build fetches, which is Table III;
* a flow the build runs, such as synthesis or a layout.

Four things in that document are counts and not opinions, and each has
a command behind it.
Run them; do not carry a number over from the last edit.

```sh
# What `bazel test //...` runs is the targets less the manual ones,
# which is the number the document quotes; the tail of a test run
# says the same thing.
bazel query 'kind(".*_test", //...)' | wc -l
bazel query 'attr(tags, "manual", kind(".*_test", //...))' | wc -l
bazel query 'kind(".*_test", //...)' | grep -c _vsim    # netlists, each x2
find lib/parts/src -name '*.rs' | xargs wc -l | tail -1 # a band of lines
ls lib/examples/ex_*.rs | wc -l                         # the examples
```

The document stays within five pages.
That is the constraint that makes it worth reading, so when an
addition pushes it onto a sixth, something else gives: a table of
numbers becomes a sentence, or a paragraph that has stopped earning
its place goes.
Two tables have already been folded into prose that way.
Check with `pdfinfo bazel-bin/docs/showcase.pdf`, and render every
page and look at it, as every document change here requires.

The same applies to `//docs:cover`, which names every document, and to
the list of documents in this file: a new document is added to both in
the change that creates it.

# Standing rule: every bug found is filed

A bug found along the way is filed as an issue on `HDL/txhdl` when it
is found, whatever the task at hand was.
That covers the library, the lowering and the netlists, the parts, the
cores, the tools, the build and the documents.
A bug that is worked around and not filed is paid for again by the
next person to meet it, and nothing records the workaround's reason.

The issue says:

* what goes wrong, with the smallest reproduction that shows it, and
  the output or the error it gives;
* what should happen instead;
* how it was found, naming the issue or change that was under way;
* the workaround, if one was used, and where it is.

A workaround in the tree names its issue in the comment beside it, and
the commit that brings the workaround names the issue too.
Before filing, look for an open issue that already covers the bug; if
one does, add what was learned to it rather than filing another.
Report the issues filed when reporting on the task.

# Standing rule: every component has a datasheet

A component is a unit under `#[lower]` outside `lib/examples`, or a
family of units a macro writes, such as `router!`'s routers.
When a component is added, its datasheet is added in the same change,
or the change is not finished.
When a component's parameters, ports, register map or behaviour
change, its datasheet is updated in the same change.

1. Add `docs/datasheets/<Key>.tex`, following the other sheets: a
   `% covers:` line naming every type the sheet covers, then
   `\datasheet`, `\dsfacts`, the function, the parameters, any
   register or address map, `\dstables{<Key>}`, the behaviour, the
   verification with its test targets, where it is used, and its
   limits.
   Say nothing on a sheet that the code, its tests or a document does
   not state.
2. Add `\input{datasheets/<Key>}` to `docs/datasheets.tex`.
3. Add the component to `tools/datasheet/main.rs`, lowered with the
   parameters the tree uses it with, under the same key.
   The document does not build if a sheet asks for tables the
   generator does not write.
4. `//tools/datasheet:coverage_test` fails when a component in the
   crates it scans has no sheet, or a sheet is not in the document.
   A new crate with lowered units adds its `sources` to that test's
   `data` and its source directory to the test script's `find`.

# Standing rule: one issue, one pull request, in topical commits

An issue is a pull request of its own.
Two issues in one branch cannot be reviewed apart, merged apart or
reverted apart, and the one that turns out to be wrong holds up the
one that is right.
A pull request that closes three issues is three pull requests that
were not made.

Inside a pull request the commits are topical, not chronological.
A part, the example that checks it, and the documents that describe it
are three commits and not one, because a reviewer reads them
differently: the first is hardware, the second is a test, the third is
prose.
The order to commit in is the order of the three places rule above,
which is also the order to work in.

Four things follow.

* Every commit builds.
  `bazel build //...` is green at each one, so that a bisect is worth
  running and a commit can be reverted on its own.
* A change the work did not ask for is a commit of its own, named as
  such.
  What a formatter does to a file the change never touched belongs
  there, not carried along by the feature that ran the formatter.
* A bug found along the way is its own commit, naming the issue it
  closes or works around, as the rule above requires.
* A commit that only moves or renames is separate from one that
  changes what moved, because the two together are unreadable as a
  diff.

When one issue's work genuinely needs another's, the branch is stacked
on it rather than merged into it, and the pull request says so in its
first line.

## A pull request is watched until it lands

Opening a pull request is not the end of the work on it.
`main` moves while a branch waits, and a branch that has gone stale is
a branch nobody can merge without doing the author's work for them.

* Check for conflicts as soon as the pull request is opened, not only
  later.
  A branch can conflict the moment it is pushed, because `main` may
  have moved between the last rebase and the push.
* Check every open pull request again whenever `main` moves, and
  before reporting on a task.
  One command says it for all of them:

```sh
fj pr -R hd search -s open      # the numbers
fj pr -R hd status <number>     # mergeable, or not
```

* Repair a conflict by rebasing onto `origin/main`, not by merging
  `main` into the branch.
  A rebase keeps the topical commits topical; a merge buries them
  under a commit that is about nothing.
* A stacked branch is rebased onto its parent after the parent moves,
  in that order, parent first.
* After a rebase, the checks are run again.
  A conflict resolved by hand is a change nobody has built, and `main`
  may have added a rule the branch does not yet meet: the datasheet
  test above is exactly that kind of rule.

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
bazel build //cpu/vreteno/asic:vreteno_cells  # the core onto standard cells
bazel build //cpu/vreteno/asic:vreteno_pnr    # and to a layout; manual, 20 min
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
