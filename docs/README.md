<!-- SPDX-License-Identifier: Apache-2.0 -->
# Documents

This directory holds the material that outlives the integration of the two
language specifications into one.

The language is now the library, `//lib`, and the documents typeset here
describe it.
It began stated twice, under two names, in two places, and those are its
history: `spec/language.md` stated TxHDL, and `filmil/workspace/` stated
LHDL, across four files that disagree with each other.
Consolidating them meant choosing, and a choice is worth nothing if the
reasons for it are lost.
Those reasons live here.

## Contents

| File | What it holds |
|---|---|
| `unification-analysis.md` | What each language is, what conflicts, what each contributes, and every defect found in the two sources |
| `syntax-decisions.md` | The Rust-shaped surface syntax chosen for the merged language, one decision per conflict, with a worked example |
| `document-build-plan.md` | How the articles are built, what was tried before, and why the TeX packages are vendored |
| `rust-embedding.md` | Working notes on embedding the language in Rust, with every probe result in full |
| `on-chip-debug.md` | What the board shows today, the options for debugging the SoC on it, what each costs, and the order to do them in |
| `vivado-setup.md` | Vivado for this repository from nothing: why the ordinary build needs none, the hermetic installation and the host one, and what goes in your own `user.bazelrc` |
| `noc-bursts.md` | Why a multi-beat write did not cross the network on chip, which of the stated reasons survived measurement, what each candidate design cost in bits, which one was chosen, and how it was built (section 7) |
| `opengl-gap.md` | What Razboj does against what OpenGL ES 1.1 and a subset of 2.0 ask for, the gaps sized, a staged path from Razboj on the board to a fixed-function GL, and the issues to file for it (issue 926) |
| `ddr3-throughput.md` | What limits the shared path into DDR3, measured in simulation, and three ways to widen it, with the recommendation and the order of the work (issue 1023) |
| `gles.md` | The GL ES 1.1 Common-Lite library on Vreteno, designed before its language is chosen: the API subset, the pipeline on the CPU, the fixed-point formats against Razboj's display list, the frame and the tiles, the icosahedron as the first program, and the language choice for the user (issue 995) |
| `razboj-tiles.md` | Razboj drawing in tiles, as the user decided: the tile size, the tile buffer in block RAM, binning on the CPU, finished tiles to DDR3 in bursts, the block RAM budget, and the order to build it in (issue 991) |
| `board-checks.md` | Every board check the issues owe, as commands in session order: the bitstreams to build first, what each run should print, what to keep, and which issue it closes |
| `demos.md` | The canonical demos redone after each bigger milestone: the list in `demos.tsv`, the tool that runs them on the board, and where the videos go (issue 1592) |
| `sv32-timing.md` | Where Sv32 translation sits in the core, the walker and the TLB sizes, the core's two critical paths measured with a prototype TLB and with operands read at the edge, the cost in cycles of each option, and the recommendation (issue 1009) |

The documents typeset here are the ones `cover.tex` names:
`article.tex` with `sections/` is the merge, `//docs:article`;
`embedding.tex` with `embedding_sections/` is the language, `//docs:embedding`;
`runtime.tex` with `runtime_sections/` is the runtime library verbatim, `//docs:runtime`;
`examples.tex` with `examples_sections/` is every example with its output, `//docs:examples`;
`vreteno.tex` is the Vreteno RV32IMAC core, `//docs:vreteno`;
`tapeout.tex` is the core through an open ASIC flow, `//docs:tapeout`;
`paper.tex` is the expository paper, `//docs:paper`;
`cover.tex` is the cover, `//docs:cover`;
`showcase.tex` is at most five pages on the whole of it, `//docs:showcase`;
`cheatsheet.tex` is the two-sided cheat sheet, `//docs:cheatsheet` and `//docs:cheatsheet_png` (a PNG per side);
`tutorial.tex` is the tutorial, `//docs:tutorial`;
`zero.tex` is the tutorial from an empty directory, `//docs:zero`, on `tutorial/blinky/`;
`prove.tex` is the tutorial on formal verification, `//docs:prove`, the digit proved, broken and covered;
`station.tex` is the tutorial on the reservation station, `//docs:station`;
`axi.tex` is the tutorial on the AXI link, `//docs:axi`, and holds its examples;
`razboj.tex` is Razboj, the minimal GPU on that link, `//docs:razboj`, with the picture it drew;
`noc.tex` is the network on chip, `//docs:noc`, nodes on a lattice with AXI at their exits;
`eth.tex` is the Ethernet part and the echo design on the board, `//docs:eth`;
`hdmi.tex` is the HDMI part and its demonstration on the board, `//docs:hdmi`;
`flagship.tex` is the one bitstream that holds every part proven on the board, `//docs:flagship`;
`pcie.tex` is PCIe on the board, `//docs:pcie`;
`datasheets.tex` with `datasheets/` is a datasheet per component, `//docs:datasheets`;
`stats.tex` is the repository in numbers, a snapshot, `//docs:stats`;
`dynamics.tex` is how the project was built, with its timeline chart, `//docs:dynamics`;
`//docs:all` concatenates them all into `txhdl.pdf`, a bookmark each, in
the order of `DOCUMENTS` in `BUILD.bazel`.
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
