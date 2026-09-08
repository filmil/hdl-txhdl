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
| `document-build-plan.md` | How the article is built, what was tried before, and why two TeX packages are vendored |

`article.tex` and `sections/` are the article itself, built by
`//docs:article`.
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
