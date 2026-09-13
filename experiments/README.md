<!-- SPDX-License-Identifier: Apache-2.0 -->
# experiments

Compiled probes. Each one answers a question a design document would
otherwise have to argue about.

A probe is not a test of this project's code. It is a test of what another
language or tool will accept, kept in the build so that the answer stays
true. When a Rust release changes an answer, a probe breaks and somebody
sees it, rather than a paragraph quietly becoming wrong.

| Directory | Question it explores |
|---|---|
| `rust_embedding/` | What remains of TxHDL once every construct Rust already provides is deleted |

## Probes that are meant to fail

A probe whose answer is "no" is kept, not deleted, because the failure is
the answer and the error message is the evidence.
Those targets are tagged `manual`, so `bazel build //...` does not try
them.
Build one by naming it:

```sh
bazel build //experiments/rust_embedding:probe_widths          # expect failure
bazel build //experiments/rust_embedding:probe_processes_mut   # expect failure
```

`probe_widths_nightly` is also tagged `manual`, for a different reason: it
needs a toolchain flag.

```sh
bazel build //experiments/rust_embedding:probe_widths_nightly \
  --@rules_rust//rust/toolchain/channel=nightly
```

Every result is tabulated in `docs/rust-embedding.md` and in the article
built by `//docs:embedding`.
