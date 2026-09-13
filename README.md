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

The specification is `spec/language.md`.
`docs/` holds the analysis and the decisions behind it, and
`docs/README.md` says what is in there.

## Mirror

The mainline is mirrored to GitHub, `filmil/hdl-txhdl`, by the `mirror` workflow.
It runs on every push to `main` and every six hours, and needs the Actions secret `A_GITHUB_MIRROR_TOKEN`, a fine-grained token with write access to that repository's contents.
The instance's own push mirror does the same from the server; either is enough.

## Building

Everything is built by Bazel, and Bazel provisions every tool it uses.
Nothing has to be installed first beyond `bazelisk`.

```sh
bazel build //...                 # every document
bazel test  //...
```

The first build fetches a Debian rootfs, a Zig toolchain, graphviz built
from source and a TeX installation, which is roughly 950 actions.
Later builds reuse them.

## The board

The Vreteno core's board, an Alinx AX7A200, sits on another machine
with its programming cable and its serial bridge, and is programmed
from here over ssh; `.bazelrc` names the machine in `TXHDL_BOARD_SERVER`,
and `--server=HOST` on either command below overrides it.
Vivado's `hw_server` and the cable's libraries come out of the hermetic
Vivado into a bundle; the first command uploads it, starts it there and
tunnels its port back as `localhost:3122`; the second has Vivado connect
to the tunnel and write the FPGA; the third watches the serial port
there for a while and types a reply once the board has said a line.
Start the watcher before programming, since the board speaks at once.

```sh
bazel run //cpu/vreteno/board/remote:hw_server
bazel run //cpu/vreteno/board/remote:serial -- --seconds=60 --reply=yes &
bazel run //cpu/vreteno:vreteno_board_prog -- --hostport localhost:3122 --device '*/xilinx_tcf/Digilent/*'
```

The board answers `OK`, then echoes `yes`.
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
