<!-- SPDX-License-Identifier: Apache-2.0 -->
[![build](https://git.hdlfactory.com/HDL/txhdl/actions/workflows/build.yml/badge.svg?branch=main)](https://git.hdlfactory.com/HDL/txhdl/actions?workflow=build.yml)
[![release](https://git.hdlfactory.com/HDL/txhdl/actions/workflows/release.yml/badge.svg)](https://git.hdlfactory.com/HDL/txhdl/releases)

# TxHDL

A transaction-centric hardware description language, proven on
hardware: the Vreteno RISC-V core written in it runs on an Artix-7
board and says on its serial port what its simulation says.

This project is collaboration between Filip (filmil) and Dragiša (dj3maj)
The authors used a large language model, Claude, as an assistant in
exploring the concepts and in writing and constructing the documents and
the programs; every commit says so and carries its prompts verbatim.

The language is the library, `//lib`, and the documents under `//docs`
describe it; `//docs:cover` names every one and says which to read for
what.
`spec/language.md` is the specification the project started from, of
2026-05-04, kept as history: its standalone syntax is not what the
library implements, and nothing builds it.
`docs/` also holds the analysis and the decisions behind the language,
and `docs/README.md` says what is in there.

## Mirror

The mainline is mirrored to GitHub, `filmil/hdl-txhdl`, by the `mirror` workflow.
It runs on every push to `main` and every six hours, and needs the Actions secret `A_GITHUB_MIRROR_TOKEN`, a fine-grained token with write access to that repository's contents.
The instance's own push mirror does the same from the server; either is enough.

## Starting from nothing

`//docs:zero` is the tutorial for an empty directory: five files,
under `tutorial/blinky/`, that build, run and lower a blinky with
Bazel against the GitHub mirror, with Bazelisk and Git installed and
nothing else.
The tree builds the blinky against its own library as a check; the
workspace's pin is checked by building it outside the tree.

## Building

Everything is built by Bazel, and Bazel fetches the tools it uses at
the versions `MODULE.bazel` pins: the Rust toolchain, TeX, the two
simulators, the RISC-V compiler and Zephyr's tools; Vivado it installs
from AMD's installer, only when a target asks for it.
The C and C++ toolchain is one of them: LLVM, over a Debian sysroot
unpacked from packages pinned by checksum, so neither the compiler nor
the C library comes from the machine (issue 782).
What the machine needs is `bazelisk`, and Git to clone.

```sh
bazel build //...                 # every document
bazel test  //...
```

The first build fetches those tools and builds nvc from source; later
builds reuse them.

A test that reports `(cached)` has the result its exact inputs gave
before, which may have been in another worktree or in CI: on this
project's host every build shares one disk cache, so a suite can come
back green in seconds after a rebase without running anything.
`tools/suite.sh` runs the suite and says how many results ran here,
how many came from this checkout's cache and how many from the shared
one; `tools/suite.sh --fresh` runs every test here.

Neither command wants Vivado: every target that needs it is `manual`
and runs only when asked for by name.
When you do want one, synthesis, place and route or the board's
simulation, `docs/vivado-setup.md` says what to do from nothing,
including what belongs in your own `user.bazelrc`.

## Silicon

The Vreteno core's netlist, the same file the core's own FPGA build
reads, also goes through an open ASIC flow, onto the Nangate45
standard cell library. The board's bitstream is built from the whole
board, of which the core is one part.
Yosys and OpenROAD are fetched by checksum as Debian packages and
unpacked by the build, so nothing is installed for this either.

```sh
bazel build //cpu/vreteno/asic:vreteno_cells   # onto standard cells
bazel build //cpu/vreteno/asic:vreteno_pnr     # a layout; manual, 20 min
```

`//docs:tapeout` has the flow, the layout, the numbers and a list of
everything a real tapeout would still want.

## The board

The Vreteno core's board, an Alinx AX7A200B, sits on another machine
with its programming cable and its serial bridge, and is programmed
from here over ssh; `.bazelrc` names the machine in `TXHDL_BOARD_SERVER`,
and `--server=HOST` on either command below overrides it.
Vivado's `hw_server` and the cable's libraries come out of the hermetic
Vivado into a bundle; the first command uploads it, starts it there and
tunnels its port back as `localhost:3122`; the second watches the serial
port there for a while, in the background; the third has Vivado connect
to the tunnel and write the FPGA.
Start the watcher before programming: the board's program prints its
first line within seconds of the part being configured, and a watcher
opened afterwards can miss it.
On 2026-10-04 a watcher opened before `vreteno_board_prog` received
the DDR3 test whole (#965); issue 195 had once found that a watcher
held open across a reconfiguration received nothing, and that no
longer holds.

```sh
bazel run //cpu/vreteno/board/remote:hw_server
bazel run //cpu/vreteno/board/remote:serial -- --seconds=180 &
bazel run //cpu/vreteno:vreteno_board_prog -- --hostport localhost:3122 --device '*/xilinx_tcf/Digilent/*'
```

The bitstream holds the DDR3 test: it says `vreteno ddr3 test`, a dot
every ten seconds for half a minute, `ddr3 ok` once every word came
back, and dots for two minutes after, so a watcher opened a few
seconds late still reads the verdict.
`//cpu/vreteno:vreteno_board_flash` writes the QSPI flash the same way,
so the design survives a power cycle.

## Releases

`release` publishes the rendered documents.
It runs every night and overwrites the rolling `nightly` release.
When the `A_GITHUB_MIRROR_TOKEN` secret is set, the same release goes to
the GitHub mirror too, under the same tag; without it that step is skipped.
Running it by hand instead cuts a `release-YYYYMMDD-HHMMSS` release that
nothing later overwrites:

```sh
fj -H git.hdlfactory.com actions dispatch release.yml main -R origin
fj -H git.hdlfactory.com release list -R origin
```

## Before pushing

Every build checks every Rust target's formatting, through the rustfmt aspect `.bazelrc` turns on beside Clippy's (issue 1152).
A push checks what a targeted test run leaves out: the formatting of every Rust target, with `bazel build --output_groups=rustfmt_checks //...`, then `//:fmt_test`, and every document's words on their page, `//docs:edge_test`.
`tools/hooks/pre-push` runs them and refuses the push if any fails, naming the fix (issue 1133).
Turn it on once per clone:

```sh
git config core.hooksPath tools/hooks
```

That setting lives in the clone's `.git/config`, so it applies to every worktree of the clone.
The path is relative, so each worktree runs the hook from its own checkout; a worktree whose base predates the hook has none, and pushes as before.
When nothing has changed since the last build both tests come from the cache in seconds; a change to a document or to code a document includes typesets it again first.
`git push --no-verify` skips the hook, and is for emergencies only.

## Running the workflows locally

The workflows live in `.forgejo/workflows`, because the canonical remote is
Forgejo.
There is nothing under `.github`, and a checkout that still has one is out
of date: fetch and reset before testing, or `act` will run a workflow that
was deleted.

Forgejo Actions is act underneath, so `act` reproduces a run locally.
Two flags are needed every time.
`-W` names the directory, because act reads `.github/workflows` by default.
`-P` maps the `docker` label this instance's runners advertise onto a real
image, because act knows nothing about that label.

```sh
act -W .forgejo/workflows -P docker=catthehacker/ubuntu:act-latest pull_request
act -W .forgejo/workflows/release.yml -P docker=catthehacker/ubuntu:act-latest workflow_dispatch
```

Name the event, as those commands do.
`release.yml` triggers only on `schedule` and `workflow_dispatch`, so act's
default `push` event matches nothing and runs no job at all.

A local release run reaches the final step and fails there, because
publishing needs a token that a local run does not have.
Everything before it is the part worth testing.

## License

Everything in this tree outside `third_party/` is under the Apache
License, version 2.0, the text of which is in `LICENSE`.
Every source file says so in an `SPDX-License-Identifier` line.
`third_party/` holds vendored files under their own licenses, each
named in `third_party/README.md`.
