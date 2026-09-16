<!-- SPDX-License-Identifier: Apache-2.0 -->
# Documents

This directory holds the material that outlives the integration of the two
language specifications into one.

The repository currently states the language twice, under two names, in two
places.
`spec/language.md` states TxHDL.
`filmil/workspace/` states LHDL, across four files that disagree with each
other.
Consolidating them means choosing, and a choice is worth nothing if the
reasons for it are lost.
Those reasons live here.

## Contents

| File | What it holds |
|---|---|
| `unification-analysis.md` | What each language is, what conflicts, what each contributes, and every defect found in the two sources |
| `syntax-decisions.md` | The Rust-shaped surface syntax chosen for the merged language, one decision per conflict, with a worked example |
| `document-build-plan.md` | How the articles are built, what was tried before, and why the TeX packages are vendored |
| `rust-embedding.md` | Working notes on embedding the language in Rust, with every probe result in full |

Fourteen documents are typeset here, and `cover.tex` names them all:
`article.tex` with `sections/` is the merge, `//docs:article`;
`embedding.tex` with `embedding_sections/` is the language, `//docs:embedding`;
`runtime.tex` with `runtime_sections/` is the runtime library verbatim, `//docs:runtime`;
`examples.tex` with `examples_sections/` is every example with its output, `//docs:examples`;
`vreteno.tex` is the Vreteno RV32I core, `//docs:vreteno`;
`paper.tex` is the expository paper, `//docs:paper`;
`cover.tex` is the cover, `//docs:cover`;
`cheatsheet.tex` is the one-page cheat sheet, `//docs:cheatsheet` and `//docs:cheatsheet_png`;
`tutorial.tex` is the tutorial, `//docs:tutorial`;
`zero.tex` is the tutorial from an empty directory, `//docs:zero`, on `tutorial/blinky/`;
`station.tex` is the tutorial on the reservation station, `//docs:station`;
`axi.tex` is the tutorial on the AXI link, `//docs:axi`, and holds its examples;
`gpu.tex` is the minimal GPU on that link, `//docs:gpu`, with the picture it drew;
`stats.tex` is the repository in numbers, a snapshot, `//docs:stats`;
`//docs:all` concatenates them all, in that order, into `txhdl.pdf`.
`housestyle.tex` is the preamble they all share, so they cannot drift apart
in font, listing style or figure style.
The examples document shows the runtime only through its public interface,
which `//tools/api` extracts from the source at build time.
Its waveform figures are drawn by the build: `vcdcvt` and `sqlite2drawtiming`
(prebuilt, pinned in `//:multitool.lock.json`) turn the FST an example wrote
into drawtiming text, and `//tools/dt2tikz` draws that text as TikZ.
The class and package files it needs are vendored under `//third_party`, and
a `genrule` here copies them into this package, because LaTeX cannot read
them where they live.
`document-build-plan.md` section 3 says why they are vendored at all.

## What belongs here

A document belongs in `docs/` when it stays useful after the merged
specification exists.
Three kinds qualify:

* A decision and the reason behind it.
  The merged specification states what the language is.
  It does not state why `=` beat `:=`, and somebody will ask.
* An analysis of source material that the merge consumes.
  Once `filmil/workspace/draft-spec.md` is folded in and deleted, the
  record of what it said is here.
* A plan for machinery that the specification feeds, such as the document
  build.

A document does not belong here when the specification itself should state
it.
Grammar, semantics, and examples go in the specification.
