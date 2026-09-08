# Building an IEEEtran article from the specification

Status: plan, September 8, 2026.
Author: automated coding assistant, with human supervision.

The request is machinery that generates IEEEtran articles from the
specification.
This document says what to build, in what order, and what has already been
checked on the build host.


## 1. The article is not the specification

Start here, because building the wrong thing is the main risk.

A language specification is a reference.
A reader arrives knowing what to look up, and leaves as soon as the answer is
found.
It states every construct, in a fixed order, with a grammar.
`spec/language.md` runs 2241 lines and 20 sections, and that is the right
shape for it.

An IEEE article is an argument.
A reader arrives knowing nothing and reads front to back once.
It runs 10 to 18 pages in two columns, picks one running example, and threads
it through every section.
Nobody reads a 2241 line reference in two columns, and no reviewer accepts a
paper that lists every bit manipulation builtin.

So the repository builds two documents from one body of material:

| Document | Source | Format | Built by |
|---|---|---|---|
| Language reference | `spec/language.md` | single column PDF and HTML | `bazel_ebook`, already a dependency |
| Article | `article/article.tex` and `article/sections/*.tex` | IEEEtran two column PDF | `rules_latex_host`, to be added |

The article's argument already exists.
`filmil/theses.md` states 12 theses, and section 7 of
`unification-analysis.md` recommends folding them into the specification as
a rationale section.
They are also the paper's spine.


## 2. How the two documents stay in agreement

Two documents describing one language will drift, and the code examples drift
first.
The fix is that neither document owns an example.

```
   examples/
     mac_unit.tx          <-- the only copy of this code
     wishbone.tx
     dma.tx
        |                         |
        | \lstinputlisting        | pandoc include filter
        v                         v
   article/sections/*.tex    spec/language.md
        |                         |
        v                         v
   IEEEtran PDF              reference PDF and HTML
```

Three things follow from this, and the third is the one that matters:

1. An example is written once.
   Editing `examples/mac_unit.tx` updates both documents.
2. The article and the reference cannot show different code for the same
   construct, because there is only one copy.
3. The examples become testable.
   `fragments/eng-standards.md` in the coding SOP requires that every piece
   of specification has a test that passes.
   A specification written as prose cannot satisfy that rule.
   A specification whose examples are files in a directory can: when the
   parser exists, the test is that every file in `examples/` parses, and
   every construct in the grammar appears in at least one file.

Build this before writing the article, not after.
Retrofitting shared examples means editing both documents by hand once.


## 3. Work items

### 3.1 Add the LaTeX toolchain

`MODULE.bazel` gains one dependency.
The custom registry is already first in `.bazelrc`, so nothing else changes
there:

```python
bazel_dep(name = "rules_latex_host", version = "0.3.0")
```

`0.3.0` is the latest version published in the registry as of
September 8, 2026.
Check before using it, because this document ages:

```sh
curl -s https://raw.githubusercontent.com/filmil/bazel-registry/main/modules/rules_latex_host/metadata.json
```

`.bazelrc` gains two lines, because the toolchain shells out to the host's
TeX installation:

```
build --spawn_strategy=local
build --verbose_failures
```

This is a real departure from the `hermetic` fragment of the coding SOP,
which requires that every tool come from `MODULE.bazel` and nothing from the
host.
`rules_latex_host` takes `pdflatex`, `poppler-utils`, and `ghostscript` from
the host by design.
State the tradeoff in `AGENTS.md` rather than leaving a reader to find it.
A full TeX Live distribution vendored into the build runs to roughly 5 GB,
and the article is the only target that needs it.
Install the tools once with:

```sh
bazel run @rules_latex_host//latex:install_tools
```

**Already checked on this host.**
`pdflatex`, `pdfinfo`, `pdftoppm`, and `gs` are all present under `/usr/bin`.
`IEEEtran.cls` is present at
`/usr/share/texlive/texmf-dist/tex/latex/ieeetran/IEEEtran.cls`.
`pandoc` is **not** installed, which does not block anything, because
`bazel_ebook` provisions its own.

### 3.2 Raise the Bazel pin

`.bazelversion` reads `9.0.1`.
The `bazel` fragment of the coding SOP requires `9.2.0` or later, because
`9.1.0` and earlier crash with a `NullPointerException` when fetching a
repository named by a `file://` URL.
`ProgressInputStream.reportProgress` calls `String.equals` on
`URI.getHost()`, which returns `null` for such a URL.
Issue https://git.hdlfactory.com/HDL/txhdl/issues/1 tracks this alongside the
workflow work.

### 3.3 Lay out the article

Copy `templates/master.tex` from the `latex-pdf-tutorial` skill and fill in
the title, `\thanks`, abstract, keywords, and the section list.
The house preamble it already encodes is
`\documentclass[journal,9pt]{IEEEtran}`, lmodern with `T1` encoding,
microtype, amsmath, tikz, pgfplots, booktabs, enumitem, and hyperref.

```
article/
  article.tex          master: title, abstract, \input list, thebibliography
  BUILD.bazel          latex_document(name, main, deps = glob(["sections/*.tex"]))
  sections/
    01_introduction.tex
    02_why_a_new_language.tex     <- from filmil/theses.md
    03_running_example.tex
    ...
    NN_conclusion.tex
```

Each section file holds exactly one `\section{...}` with a `\label{sec:...}`
on the next line.
Write the introduction and the conclusion by hand.
The introduction opens with `\IEEEPARstart`, names the running example, and
holds a "how to read" subsection.

Add `listings` and define an `\lstdefinestyle` for the language, since the
article is nothing but code examples.
Do not make `'` a string delimiter, because that breaks any construct that
uses a single quote, and Rust-style labels such as `'outer` are in the merged
syntax per `syntax-decisions.md`.

### 3.4 Wire the build

```python
load("@rules_latex_host//latex:defs.bzl", "latex_document")

latex_document(
    name = "article",
    main = "article.tex",
    deps = glob(["sections/*.tex", "../examples/*.tx"]) + ["//:buildstamp"],
)
```

Add the build stamp block to the master before `\title{`:

```latex
\InputIfFileExists{buildstamp.tex}{}{}
\providecommand{\buildstamp}{Draft build (outside the Bazel flow).}
```

and close the `\author` with `}\thanks{\buildstamp}}`.
Append it, never nest it.
A nested `\thanks` fails with "Missing control sequence inserted."

Never commit the PDF.
`.gitignore` already excludes `bazel-*`, and the artifact is delivered from
`bazel-bin/article/article.pdf`.

### 3.5 Give the reference a target

`spec/language.md` has no Bazel target today, so the largest document in the
repository is never built and never published.
`filmil/workspace/BUILD.bazel` shows the pattern to copy: a `markdown_lib`,
an `ebook_pdf`, and a `pandoc_standalone_html` per document.
Add `spec/BUILD.bazel` with the same three rules.

Do this before the article.
It is ten lines, and it fixes the larger of the two problems.


## 4. Checks that the build cannot do for you

The `latex-pdf-tutorial` skill states these in full.
Three matter enough to repeat, because each fails silently.

**The dash ban.**
No `---` and no `--` in prose, anywhere.
Write a range with "to".
The build is green either way, and the reader is the one who notices.

```sh
grep -n -- '---\|[^-]--[^-]' article/sections/*.tex article/article.tex
```

**Figure occlusion.**
A TikZ arrowhead buried in a box border produces no warning and no error.
The document compiles clean, every reference resolves, and the figure is
still wrong.
Find the pages holding figures, render them, and look at them:

```sh
PDF=bazel-bin/article/article.pdf
N=$(pdfinfo $PDF | awk '/Pages/{print $2}')
for p in $(seq 1 $N); do
  pdftotext -f $p -l $p $PDF - 2>/dev/null | grep -q '^Fig\.' && echo "figure on page $p"
done
pdftoppm -r 200 -png -f <p> -l <p> $PDF /tmp/fig
```

Set `shorten >=2.5pt, shorten <=2.5pt` on the shared arrow styles, then check
that every `node distance` is well above twice that.
2.5 pt at each end consumes about 1.8 mm of the gap, so a row of boxes at
`node distance=1.5mm` ends up with no visible arrow at all.

**Overfull boxes and undefined references.**
Run `pdflatex` twice, then count:

```sh
grep -cE 'Overfull \\hbox \([4-9][0-9]\.|Overfull \\hbox \([0-9]{3}' log   # target 0
grep -cE 'Reference .* undefined|Citation .* undefined' log                # target 0
```

Two more from the skill's list apply directly here.
`siunitx` is not installed, so write units as plain text: `100\,MHz`.
Escape LaTeX specials inside `\code{}` and `\texttt{}` in prose.
The merged syntax uses `#[tag(..)]` and `$clog2` appears in both sources, and
a bare `$` opens math mode and fails the build.


## 5. Suggested order

1. Add `spec/BUILD.bazel`, so the specification is built and published.
   Ten lines, and independent of everything else.
2. Raise `.bazelversion` to `9.2.0`.
3. Create `examples/` and move the code out of both documents into it.
   Do this before either document is rewritten.
4. Add `rules_latex_host` and the two `.bazelrc` lines, and build a one page
   `article/article.tex` that says nothing.
   A working empty build is worth more than a written unbuilt article.
5. Write the article, section by section, against the merged specification.
6. Run the readability pass, the dash scan, and the occlusion pass.
7. Add the article target to the release workflow described in issue
   https://git.hdlfactory.com/HDL/txhdl/issues/1, so a nightly build
   publishes the PDF.

Step 4 before step 5 is the one to insist on.
The article cannot be written against a specification that does not exist
yet, and the build can be proven against a document that says nothing.
