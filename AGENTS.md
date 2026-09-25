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
The language is a library, `//lib`, and twenty-three documents describe it,
all under `//docs`:

* `//docs:cover` names every document and says which to read for what.
  A new document is added there in the same change that creates it.
* `//docs:showcase` is at most five pages on the whole of it: the
  rule, the parts, Vreteno, Razboj, how it is all checked, and what is
  not done. It is the one document meant to be read first and alone,
  so it stays within five pages and every number in it comes from a
  count rather than from memory. Its tables are as of the last sweep of
  the documents rather than of the last commit; the standing rule below
  says why, and how a sweep is done.
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
  the DDR3 memory of `//ddr3` behind it, AMD's MIG controller a
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
* `//docs:tutorial` is the tutorial, eight steps on the examples for a
  reader new to the language, and a ninth on adding one; every listing
  is included from the examples and every printout is the build's.
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
* `//docs:flagship` is the flagship: the one bitstream that holds every
  part of the system proven on the AX7A200B, kept in the board's QSPI
  flash so that the board powered on with nothing attached is the whole
  system. The Vreteno board with its DDR3, the Ethernet echo and the
  HDMI output in one part, three clock generators off one input, the
  five AXI-Lite channels of the core's third slot crossed to the pixel
  clock by `chan_cdc` in `//lib/board`, and the loader in the core's
  boot memory, so that the software on the flagship changes in a second
  and the bitstream underneath it does not move.
  `//flagship:flagship_synth`, `//flagship:flagship_pnr` and
  `//flagship:flagship_flash` are the targets.
  The first program sent down the wire is `cpu/vreteno/rust/ico_hdmi.rs`,
  a turning icosahedron the core draws into the video peripheral with
  the TxHDL logo in a corner; every picture this system generates
  carries that logo, which `//lib/logo` holds as data and
  `//tools/logo2rs` writes from the PNG.
* `//docs:pcie` is PCIe on the AX7A200B: AMD's XDMA endpoint, which
  `//pcie:xdma_x2` generates with `vivado_ip`; `AxiPins` in
  `//lib/parts` under `bus::axi_pins`, which joins a host's AXI4 pins to
  the link's channels, with its example `ex_axi_pins`; and `//pcie`, the
  lowered design behind BAR1 and the hand-written top, which
  `//pcie:endpoint_synth` and `//pcie:endpoint_pnr` put through Vivado
  to a bitstream.
  `ex_axi_pins` is documented there and not in `//docs:examples`.
* `//docs:datasheets` is a datasheet per component: every unit under
  `#[lower]` outside the examples, and every family a macro writes.
  The prose of each sheet is in `docs/datasheets/<Key>.tex`; its
  tables of ports, state and children are generated at build time by
  `//tools/datasheet` from the component's own lowering.
  A component's sheet is written in a sweep rather than in the change
  that adds it; see the standing rule below.
* `//docs:cheatsheet` is the cheat sheet, two landscape pages printed
  on the two sides of one sheet, also as a PNG per side
  (`//docs:cheatsheet_png`). The front shows `ex_cheat.rs`, its
  netlist and its waveform, all produced by the build; the back is
  the vocabulary.
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
Two tests say so rather than leaving it to whoever remembers:
`//:fmt_test` over the crates, and `//lib/examples:fmt_test` over every
example, which is where the width is load bearing (issue 281).
A listing that was typed into the article rather than included from a
file is a defect, because it is the one copy nothing checks.

# Standing rule: the showcase and the datasheets are swept, not written per change

`docs/showcase.tex` and the sheets under `docs/datasheets/` are the two
files that every branch which adds a component wants to edit.
Both say something about every component, and the showcase also holds
counts taken from the whole tree, so two branches that each add one
conflict there, and the conflict is in sentences and totals that no
rebase resolves on its own.
On September 17, 2026, four branches went out in a day and the showcase
conflicted on three of them, each time in the same two lines of counts.

So a change that adds a component, or changes one, leaves both alone.
It files an issue instead and says in its pull request that it did.

* One issue for the sheet, titled
  `docs(datasheets): a sheet for <Key>`, naming the component, the file
  its code is in, and the parameters the tree uses it with.
* One issue for the showcase, titled
  `docs(showcase): <component> in the tables`, saying which table it
  belongs in and what it is.
* One issue may carry both when a change adds one component.

A component is a unit under `#[lower]` outside `lib/examples`, or a
family of units a macro writes, such as `router!`'s routers.
What belongs in the showcase is wider: a unit others can use, a system
the build assembles, a toolchain the build fetches, or a flow the build
runs, such as synthesis or a layout.

`//tools/datasheet:coverage_test`, which fails on a component with no
sheet, is therefore tagged `manual`: it stays out of `bazel test //...`
and is run when the sheets are written.
The filed issue, not the test, is what records that a sheet is owed.

## The sweep

A sweep is a change of its own, and it closes those issues.
It is worth doing when a few of those issues are open, and before a
release, since the showcase is the document read first.

For each component an issue names:

1. Add `docs/datasheets/<Key>.tex`, following the other sheets: a
   `% covers:` line naming every type the sheet covers, then
   `\datasheet`, `\dsfacts`, the function, the parameters, any
   register or address map, `\dstables{<Key>}`, the behaviour, the
   verification with its test targets, where it is used, and its
   limits.
   Say nothing on a sheet that the code, its tests or a document does
   not state.
   The diagram of the component's interfaces and the snippet that
   makes and joins it come with `\dstables`, written from the
   lowering by the generator, so a sheet neither draws nor types them
   (issue 173).
2. Add `\input{datasheets/<Key>}` to `docs/datasheets.tex`.
3. Add the component to `tools/datasheet/main.rs`, lowered with the
   parameters the tree uses it with, under the same key.
   The document does not build if a sheet asks for tables the
   generator does not write.
4. Put the component in the showcase's tables.

Then run `bazel test //tools/datasheet:coverage_test`, which is manual,
and it names every component still without a sheet and every sheet the
document does not include.
A new crate with lowered units adds its `sources` to that test's
`data` and its source directory to the test script's `find`.

Then the showcase's numbers are taken again, since a sweep is the one
place they are allowed to change.
Four of them are counts and not opinions, and each has a command
behind it.
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

The showcase stays within five pages.
That is the constraint that makes it worth reading, so when an
addition pushes it onto a sixth, something else gives: a table of
numbers becomes a sentence, or a paragraph that has stopped earning
its place goes.
Two tables have already been folded into prose that way.
Check with `pdfinfo bazel-bin/docs/showcase.pdf`, and render every
page and look at it, as every document change here requires.

Because the sweep lags the code, the showcase says that its tables are
as of the last sweep, and the open issues say what is missing.

## A document is still added in the change that creates it

None of the above applies to a new document.
`//docs:cover` names every document, and so does the list in this
file, and a new document is added to both in the change that creates
it: they are a line each, and a document arrives once.

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

# Standing rule: take the simplest issue, and say you have taken it

More than one session works on this repository at once, and two
sessions that pick the same issue do the same work twice and then
conflict over it.
So picking an issue is itself a step with a rule.

## Take it before working on it

An issue being worked on carries the `taken` label, and a comment
saying who is on it and when.
Put both on before the first line of code, not after the branch is
pushed, because the window between choosing and pushing is exactly
where two sessions collide.

```sh
fj issue -R hd search -s open       # what is open
fj issue -R hd view <number>        # the labels say what is taken
```

Skip every issue that already carries `taken`, and every issue that
carries `later`.
If the work is abandoned, the label comes off with a comment saying
why, so that the next session finds it free rather than guessing from
the silence.
The label comes off when the pull request that closes the issue is
merged, since the issue closes with it.

## Priority comes before size

An issue carrying the `priority` label is taken before anything else
that is free, whatever its size and whatever else is open.
The label is how the user says which issue matters now, and it is the
one thing that outranks the order below; size decides only among the
issues that are free and unlabelled, or among several that carry
`priority`.

```sh
fj issue -R hd search -s open -l priority   # what to take first
```

## `later` means not now

An issue carrying the `later` label is not taken, whatever else is
true of it: not because it is wrong or done, but because the user has
put it off.
It is the opposite of `priority`, and it outranks everything below in
the same way: a `later` issue is passed over even when it is the
simplest one open and even when nothing else is free.

```sh
fj issue -R hd search -s open -l later   # what is put off
```

Nothing is done to such an issue and nothing is said on it.
The label comes off when the user takes it off, and only then does the
issue join the ones a session may choose from.
If every free issue carries `later`, say so and stop rather than
taking one anyway.

## Prefer the simpler issue

Of the issues that are free and carry no `priority` and no `later`,
take the simplest one that is worth doing, not the most interesting
one.
Simplest means the one whose fix is smallest and whose check is
clearest: a one-line refusal with a probe beside it, a stale
paragraph, a dependency that may no longer be needed.

That order is not modesty, it is throughput.
A small issue lands in one pull request, in an hour, against a `main`
that has not moved under it.
A large one spends a day on a branch that goes stale twice, conflicts
with whatever else landed, and blocks nothing while it waits.
Ten small issues closed are ten issues closed; one large one half done
is none.

Reasons to pass over a simpler issue, and to say which applies:

* it carries `later`, which settles it without any reading;
* it is blocked by an issue nobody has done, and the issue says so;
* it wants hardware, a board or a cable nobody has connected;
* the user asked for a particular one, which settles it.

A large issue that is genuinely next is taken whole, not started and
left half done.
When it is unclear whether an issue is small, read it and find out
before taking it, rather than after.

## The loop

Working through the tracker is a loop, and it is the same six steps
every time.

1. **Sweep what is already done.** Before taking anything, look for
   open issues that the tree has quietly fixed, and close them with
   the evidence: the commits that did it, and the lines that show it.
   An issue that is fixed and still open sends the next session to
   work that does not exist.
2. **List and choose.** `priority` first, then the simplest free
   issue, by the rules above; an issue carrying `taken` or `later` is
   not one of them.
3. **Take it.** The `taken` label and a comment, before the first
   line of code.
4. **Do it, and check it.** The three places rule, `bazel test
   //...`, and for a document change the rendered pages.
5. **Send it.** One issue, one pull request, in topical commits, with
   `Closes #<number>` in the last of them.
6. **Look around before going again.** Every open pull request is
   checked for conflicts, and a bug found on the way is filed. Then
   step 2.

A loop that only opens pull requests is a loop that leaves them.
Step 6 is where they are kept, and it is not a glance: the pull
requests this loop has opened are its own work until they land, ahead
of the next issue.

* Check them all, not the newest.
  `main` moves under a branch opened three issues ago as easily as
  under the one opened a minute ago, and the older branch is the one
  nobody is thinking about.
* Repair a conflict when it appears, in the same turn as finding it.
  Rebase onto `origin/main`, resolve, run the checks again, force push
  with a lease, and confirm the pull request reports mergeable
  afterwards rather than assuming the push fixed it.
  Read what the conflict is before resolving it: a rule that landed on
  `main` while the branch waited can make "keep my side" the wrong
  answer, and it has been, once.
* A branch that cannot be repaired cheaply goes back to the tracker.
  Say so on the issue, take the `taken` label off, and let the work be
  picked up fresh rather than left as a stale branch nobody will
  rebase.
* **Before touching a branch that is not yours, run `git worktree
  list`.**
  The sessions here share one repository, so a branch checked out in
  another worktree is a branch somebody is standing on.
  A rebase moves the ref at once, and that worktree's `HEAD` moves
  with it, whatever its files say.
  `--force-with-lease` does not help: it guards the remote, and the
  damage is already done locally.
  A branch held by another worktree is diagnosed on its pull request
  and left alone.
  If a ref has already been moved, `git update-ref <ref> <old> <new>`
  puts it back and names the value it expects to find, so it fails
  rather than guessing.

The check itself is two commands, and the cost of skipping them is
somebody else doing the author's rebase for them.

Two things are not failures of the loop and should not be made to
look like one. An issue whose answer turns out to be "nothing to do"
is closed with what was measured and no pull request, which is a
result; #162 went that way. And an issue found to be already fixed in
step 1 is closed the same way.

## Clean up the worktrees you are finished with

A worktree costs a checkout of the tree and, once anything has been
built in it, a Bazel output base of tens of gigabytes.
Those outlive the work: a branch merges, the session ends, and the
worktree sits there full.
On September 19, 2026 there were forty-three of them on one machine
and the disk reached 97 percent, which stops every session at once.

So a worktree whose work is done is cleaned and removed, by whoever
owns it, and the order matters.

```sh
git worktree list                 # what there is, and who holds what
cd <worktree> && bazel clean      # first: this frees the build outputs
git worktree remove <worktree>    # only after the clean returns
git worktree prune                # once, at the end
```

* **Clean before removing.**
  Removing the directory first leaves the output base behind with
  nothing pointing at it, and nothing will ever clean it again.
* **Plain `bazel clean`, not `--expunge`.**
  An expunge is scoped to the workspace it runs in, so it is safe in
  a worktree of your own, but the habit is dangerous next to output
  bases that are not yours: one on this machine belongs to the
  Forgejo Vivado runner, whose warm cache took forty minutes to
  build.
  A plain clean frees the outputs, which is the point.
* **Only your own.**
  A worktree another session is standing in is not yours to remove,
  for the same reason its branch is not yours to rebase.
  Say what you propose to remove, and let the owner answer.
* **Anything dirty stays.**
  If `bazel clean` fails, or the worktree has uncommitted changes,
  leave it and say so rather than removing it.

Deciding whether a branch is finished needs the right test, because
the obvious ones all mislead here.
A merged branch's tip is usually **not** an ancestor of `origin/main`,
since pull requests land rebased; the remote branch usually still
exists, since merging does not delete it; and a diff against `main` is
large for every stale tree, since it counts what `main` gained.
What answers the question is a comparison by patch:

```sh
git cherry origin/main <branch>   # no `+` lines: all of it is in main
```

Assert that the branch resolves before trusting a zero, since
`git cherry` prints nothing when it fails and a counter reads that as
"merged".

That test is necessary and it is not sufficient, and the gap cost six
days.
It compares patch identifiers, so it answers whether a **patch**
landed and not whether the **work** did.
Anything reshaped on the way in reads as unmerged for ever:
renumbered, reworded, rebased, split, or corrected by somebody else.
On September 23, 2026 five worktrees were held on that basis and four
of them had nothing pending.
`i2c` was in `main` with its document section renumbered from `48_i2c`
to `49_i2c`; `pcie` was in `main` with `docs/pcie.tex` present;
`tuple-let` was in `main`, its text at `lib/macros/lib.rs:5888`; only
`names` was genuinely absent.
So the question was never "is the patch merged" but "is the content in
`main`".
Before calling a branch unfinished, look for its content under
whatever name it landed as.

**And a stale branch is not always a thing to preserve.**
The fifth of those, `csrs`, had an unlanded commit saying "Six more
say what the machine is" where `main` says "Five more", because that
sentence had already been corrected days earlier in a commit that
merged.
Merging that branch would have reintroduced a counting error that was
already fixed.
Every procedure here, this file included, treats an unmerged branch as
a thing to keep; sometimes it is a thing to discard, and the only way
to tell is to read what it would change.
That is the pair with the trailer rule below: in both, the obvious
signal was also the wrong one.

The dates mislead as well, and worse, because they look like facts
rather than summaries.
Rebase-merge rewrites the graph and leaves the author dates alone, so
they can run backwards against it: on September 22, 2026 a commit
authored at 05:06:48 was the descendant of one authored at 05:10:23,
and reading the times told you the opposite of the order.
On this repository only the graph is evidence about order.
`git merge-base --is-ancestor A B` answers it; a timestamp, an ahead
count and a branch's apparent age do not.

Two things worth knowing while doing this:

* A worktree's output base is named for the md5 of its path, so
  `printf '%s' <worktree path> | md5sum` finds it under Bazel's output
  root without starting a server.
  That is how to be sure which base belongs to what before deleting
  anything.
* Removing a worktree does not delete its branch, and the branch is
  usually on the remote as well, so a tree removed by mistake is
  `git worktree add` away from being back.

### Who owns a worktree is the commit trailer

The owner is the `Claude-Session` trailer on the branch's commits.
Not the branch's name, not the directory it sits in, and not a list
handed to you, including a list from the session that is
orchestrating the cleanup.

```sh
git log -1 --format='%B' <branch> | grep Claude-Session
```

On September 21, 2026 a coordinated cleanup carried a session URL
first reported on September 8 into its census as a third owner that
did not exist, and offered a live worktree to a session that was not
its owner on the grounds that it belonged to somebody else.
The trailer caught it, because the id in the trailer was the id of
the session being asked.
Check the trailer against your own id yourself, whoever tells you
what is yours.

A convention in the names is the trap, because it looks like the
check and costs nothing to believe.
The day after that census a session skipped two fully merged trees
because a list described suffixed names as a sibling's and the suffix
was read as the marker; that cost a day and 3.1 GB.
The trees happened to be finished. Had either been unfinished the
same confidence would have thrown away work, and nothing about
reading a name rather than a trailer would have felt different.

### Removing a worktree does not stop its server

`git worktree remove` leaves the Bazel server running, against an
output base whose workspace directory is now gone.
So "the worktree is gone, nothing points at the base" is not a safe
inference, and deleting the base under a live server is not a safe
act.
Read `<base>/server/server.pid.txt`, and shut the server down before
deleting the base if that pid is alive.
Two removals on September 21, 2026 left two live JVMs holding about
1.1 GB between them.

### Measuring what is on the disk

A single `du -sm /data/bazel/output` is killed for memory on this
host, and `nice` and `ionice` do not save it: they throttle processor
and disk, not memory, and it is the walk being held in memory that is
the problem.
Reaching for them is the obvious next move, and it was made twice on
the day this was written, by somebody who then reported that the
machine could not measure its own disk.
It can.
The instinct was right and the instruments were aimed at the wrong
resource, which is worth knowing before you spend an hour repeating
it.
Size one base at a time, appending as you go, and run the pass under
`setsid nohup` so that a watchdog cannot take it.
Peak memory then stays flat.

Detached is not the same as a background shell task, and the
difference showed the next day: a census run that way was killed at
171 bases of 392, by something that was not the memory killer.
So make the pass resumable as well as detached.
It records as it goes and skips what it has already recorded, and a
kill then costs only the bases it had not reached rather than the
whole measurement.
On a host that kills things, a measurement that cannot survive being
killed is not a measurement.

That is not a small difference.
On September 21, 2026 the shape above sized all 332 orphaned bases at
227.7 GB while a throttled whole-tree `du` was killed twice, and a
sample of twelve, which is what a killed pass leaves you guessing
from, came out twenty-four per cent high.

### What an orphaned base is, and which ones are not yours

An output base is orphaned when the workspace it was built for no
longer exists, which is what `--workspace_directory` in
`<base>/server/cmdline` says.

That test is what makes the rule above usable rather than a fence.
Of the 393 bases the census classified on September 21, 2026, 58 had
a live workspace, 332 did not, and 3 said nothing, and the 332 held
227.7 GB, a quarter of the disk, against the twenty or so gigabytes
that a day of removing worktrees recovered.
A bare count of the directory drifts by a base or two between reads,
since builds start and finish while you are reading, so the total to
quote is the one the classification accounted for and not the one
`ls` gave a minute later.
Most of them, 302 bases and 206.7 GB, are the Forgejo runner's
finished job bases under `/data/act`, and they are not yours to
delete however dead they look.
The runner's warm Vivado toolchain base, the one the forty minutes
went into, has a live workspace and is among the 58, so it is never
what a census of orphans is pointing at.
Say the number to whoever owns the runner and let them decide.

Measure again rather than quoting the figures above.
They are what one day looked like, and the runner's side of them
grows by a base per job unless something reclaims them, so a reader a
month later is holding a date and not a property of the machine.

### A machine that keeps killing your work may be robbed

Before concluding anything about your build, look at what else is on
the box.

On September 22, 2026 three orphaned Vivado `cs_server` processes
were found at 99 per cent each, running since the thirteenth: three
of eight cores, for nine days.
That is what killed two full test suites here, one of them already
reduced to `--jobs=1`, and what held the load at thirty to thirty
seven while the work in flight could not account for it.
The session losing the suites tuned `--jobs`, reached for `nice` and
`ionice`, and then concluded that the machine could not measure its
own disk.
All three are the wrong move, and none of them finds a thief.

What it was, because the shape is worth recognising.
The server is started with `-D`, so it daemonises into a session of
its own and a kill of the process tree it came from cannot reach it.
Its client hung up, leaving one descriptor half closed, and a poll of
a half-closed descriptor returns readable at once and for ever, so it
spins.
It is started with an idle timeout as well, which never fires,
because a process that believes it is serving traffic never becomes
idle: the fault disarms the safeguard meant to catch it.

What to look for:

* `ppid 1` together with `sid == pid`. A self-daemonised process is
  its own session leader, which is both why a tree kill misses it and
  how to end it, by session.
* Days of accumulated processor time on something nobody started this
  week.
* **`State: S` beside 99 per cent processor is not a contradiction.**
  Polling a readable descriptor is a sleep that returns instantly.
  Reading `S` as "asleep, not my problem" is how this survived nine
  days in plain sight.

The fix for the class is a cgroup and not a trap, for the same reason
the tree kill missed it: `-D` exists to escape the tree, and an
`EXIT` trap is skipped on `SIGKILL`, which is what a watchdog sends.

A **transient user service** is what does it, and not a scope, which
is the obvious reading and the wrong one:

```sh
systemd-run --user --wait --pipe --pty --collect --same-dir -- <cmd>
```

A scope is not stopped when the thing that started it exits, so a
self-daemonised child outlives it and the orphan survives exactly as
before. A transient service is torn down with its unit, children
included, and the flags above keep what a build needs: the exit
status, the environment and the working directory.
That was measured rather than reasoned, against a daemonised `sleep`,
after the scope was proposed here first and would have left the bug
in place.

Where it goes matters as much as what it is. A Bazel action in a
sandbox has no `XDG_RUNTIME_DIR` and so cannot reach the user manager
at all, which means the wrapper belongs where the process is really
spawned, at `bazel run`, and must stay silent in a sandboxed action
rather than warn on every one. A guard that cannot run where the
thing it guards is started is not a guard.

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

## Never write "does not close" beside a number

Forgejo scans a commit message and a pull request body for a number
after a closing word, and it does not read the sentence around it.
So "this does not close #411" closes #411 on the merge, silently.

The phrasing is the natural one to reach for, because a change that
does part of a job wants to say which part, and a stacked branch
always does part of a job.
Say it with the number first and no closing word near it:

* **#411 stays open**, since this is only the register map.
* Towards #390. The rest of it is the rule itself.
* Part of #151; the ring manager is not here.

Not "does not close", "does not fix", "does not resolve", nor any of
them with a number following.
The same goes for a comment on the issue, since the scan does not care
which field it read.

This was met on a pull request whose body said a change "does not
close #411" while two sessions were working through #411; the merge
would have closed it under both of them, and the issue's own history
would have been the only place it showed.

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

* Keep checking while the pull request is open, on a rhythm rather
  than once.
  Every open pull request is checked at each of these, whether or not
  it is the one being worked on: after opening one, after any pull
  request lands, before taking the next issue, and before reporting.
  A branch does not announce that it has gone stale, and the session
  that opened it is usually looking somewhere else by the time it
  does; the check is one command, and the repair is cheapest the
  moment the conflict appears, while the change is still in mind.
  This rule was written against a pull request that reported
  conflicts within the hour, on a file nobody expected to move.
* Repair a conflict by rebasing onto `origin/main`, not by merging
  `main` into the branch.
  A rebase keeps the topical commits topical; a merge buries them
  under a commit that is about nothing.
* A stacked branch is rebased onto its parent after the parent moves,
  in that order, parent first.
* After a rebase, the checks are run again.
  A conflict resolved by hand is a change nobody has built, and `main`
  may have added a rule the branch does not yet meet, as this one was
  added while four branches were open.
* **A clean merge is not a building tree.**
  `git merge-tree` and a rebase that reports no conflict both answer
  whether the texts can be combined, and neither answers whether the
  result compiles.
  A semantic conflict has no textual overlap for either to find: on
  September 22, 2026 one branch widened a tuple while another added a
  use of the old arity, the merge was clean, and `main` stopped
  building for every session at once.
  Nothing warned, because there was nothing textual to warn about.
  So build the merge, and say "it merges clean" only about merging.
  This matters most where two branches add entries to one list, a
  `srcs` in a `BUILD.bazel` say, since neither edit touches the
  other's line and the damage appears only once both have landed.

# Claims about Rust are compiled, not argued

Any claim about what Rust accepts goes into
`experiments/rust_embedding/` as a probe before it goes into a document.
A probe whose answer is "no" is kept, tagged `manual` so it stays out of
`//...`, and its error message is the evidence.
The first draft of the embedding article argued from the language
reference and was wrong twice in eleven claims.

# Prefer the interface somebody else already maintains

Where the choice is between an interface that already has maintained
software on the other side of it and one this repository would define,
take the first, and take it even when defining one looks like less
work today.
The implementation is the cheap half and the interface is the
expensive one, so the interface should be the one somebody else is
already paying to keep.

Four decisions have gone that way, and three of the four were a
session's call rather than something the user asked for:

* fastboot as the loading protocol, because the host tool ships on
  every desktop and needs nothing written for it (#143).
* Zephyr's networking for TCP, rather than a stack written here
  (#411, and #143 behind it).
* Zephyr's Ethernet driver model, so the peripheral's registers are a
  port of a driver that already works rather than a map invented here
  (#411).
* The RISC-V debug transport and debug module, so that gdb and
  OpenOCD speak to this core, rather than a debug module of our own
  (#154).

The fourth one is what made this worth writing down: the principle was
already operating, and each of us was rediscovering it and arguing it
again from the beginning.

It is a default and not a law.
An interface with maintained software behind it can still be the wrong
one, and the way to say so is to name what it costs here rather than
to reach for a fresh design because the fresh design is more
interesting.

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

* `bazel build //...` is green, which includes Clippy: `.bazelrc` runs
  the Clippy aspect on every build with its warnings as errors, so a
  lint is answered where it is written rather than found later in a
  sweep (issue 228).
  A lint the lowering forces is allowed at the item it fires on, with
  the reason beside it, and every other lint is fixed: the lowering
  reads `name: value` and not a field shorthand (issue 235), `&` and
  `>=` and not a range, and a `select!` arm's alternatives one by one.
  An `allow` goes above `#[lower]`, never between it and the `fn`,
  which hides the function from the scan (issue 234).
  `clippy::type_complexity` is allowed everywhere, in `.bazelrc`: a
  unit's ports are tuples of channel types, and a join's are tuples of
  those.
* `bazel test //...` exits 0: the lowered units agree with their traces.
* Every PDF builds, every face is Type 1, every `\ref` and `\cite`
  resolves, and no listing line overflows its frame.
* Every generated waveform fits the column it is put in:
  `waveform()` gives each diagram a `<name>_width_test` that measures
  the drawing from its coordinates and fails on a plain `figure` in a
  two-column document that it would overrun, naming the placement and
  the fix, which is `figure*` (issue 324).
* Every word of every document lies on its page: `//docs:edge_test`
  reads every rendered page back with `pdftotext -bbox`, from the
  pinned tree in `//third_party/poppler`, and fails naming the document,
  the page and the word whose box ends past the page's edge, which is
  what a table wider than its column does without a word from LaTeX
  (issue 457).
* Every page that holds a figure or a table was rendered and looked at:
  no float crosses into the other column or the margin, no float lies on
  another or on the text, and no label sits on a line or on another
  label. The build cannot see any of these.
* `CLAUDE.md` and `GEMINI.md` are symlinks to this file.
