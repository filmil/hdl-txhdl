[![build](https://git.hdlfactory.com/HDL/txhdl/actions/workflows/build.yml/badge.svg?branch=dev)](https://git.hdlfactory.com/HDL/txhdl/actions?workflow=build.yml)
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
