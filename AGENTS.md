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
The language is a library, `//lib`, and two articles describe it:

* `//docs:article` states the merge of the two languages this one came
  from.
* `//docs:embedding` states the embedding in Rust, and its appendix
  holds the runtime library and its examples verbatim.

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
3. **The appendix.** Add a `\lstinputlisting` of the example to
   `docs/embedding_sections/15_appendix.tex`, and of any new runtime
   file to the runtime section above it.
   The appendix includes the files from the tree at build time, so it
   cannot drift from the code, and it must not be written by hand.

The order is the order to work in.
Write the runtime, write the example against it, build both, then
include them.
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

```sh
bazel build //...              # the library, every example, both articles
bazel run //lib/examples:ex_config -- asic
bazel build //docs:embedding   # -> bazel-bin/docs/embedding.pdf
```

Before finishing a document change, run the readability pass from the
`prose-readability` skill, then render every page that holds a figure
and look at it.
A TikZ figure can be wrong with no warning and no error, and this
repository has had that happen twice.

# Verification

* `bazel build //...` is green.
* `bazel test //...` exits 0 or 4; 4 means no test target exists yet.
* Both PDFs build, every face is Type 1, every `\ref` and `\cite`
  resolves.
* `CLAUDE.md` and `GEMINI.md` are symlinks to this file.
