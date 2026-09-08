[![build](https://git.hdlfactory.com/HDL/txhdl/actions/workflows/build.yml/badge.svg?branch=main)](https://git.hdlfactory.com/HDL/txhdl/actions?workflow=build.yml)
[![release](https://git.hdlfactory.com/HDL/txhdl/actions/workflows/release.yml/badge.svg)](https://git.hdlfactory.com/HDL/txhdl/releases)

# TxHDL

A transaction-centric hardware description language.

This project is collaboration between Filip (filmil) and Dragisa (dj3maj)

The specification is `spec/language.md`.
`docs/` holds the analysis and the decisions behind it, and
`docs/README.md` says what is in there.

## Building

Everything is built by Bazel, and Bazel provisions every tool it uses.
Nothing has to be installed first beyond `bazelisk`.

```sh
bazel build //...                 # every document
bazel test  //...
bazel build //spec:language-pdf   # the specification as a PDF
bazel build //spec:language-html  # the specification as a web page
```

The first build fetches a Debian rootfs, a Zig toolchain, graphviz built
from source and a TeX installation, which is roughly 950 actions.
Later builds reuse them.

## Releases

`release` publishes the rendered documents.
It runs every night and overwrites the rolling `nightly` release.
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
