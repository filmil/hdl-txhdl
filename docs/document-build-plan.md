# Building the specification documents

Status: as built, September 8, 2026.
Author: automated coding assistant, with human supervision.

The specification is published as an IEEE two-column article, set in
Computer Modern, built from LaTeX by Bazel, and attached to every release.

```sh
bazel build //docs:article     # -> bazel-bin/docs/article.pdf
```

This document records what was tried, what failed, and why the build looks
the way it does.
Two earlier arrangements are described, because the reasons they were
dropped are the reasons this one is shaped as it is.


## 1. What was tried before

### bazel_ebook, Markdown to PDF through pandoc

The repository used to render Markdown with `bazel_ebook`.
Version 2.0.15 depends on `bazel_rules_bid` and shells out to `docker`, so
it failed on any machine without Docker:

```
bazel-out/.../bazel_rules_bid+/build/docker_run:
  line 198: docker: command not found
```

`bazel_ebook` 3.0.0 fixed that, and it is genuinely hermetic.
It takes pandoc and plantuml from a pinned multitool lockfile, builds
graphviz from source with `hermetic_cc_toolchain`, and gets TeX from a
`rules_distroless` rootfs pinned against `snapshot.ubuntu.com`.
It rendered a 45 page PDF of the specification here, in 591 seconds from a
cold cache.

It does not run on the project's build machine, which is the whole reason it
is gone.
The approach is sound and the implementation works.

### rules_latex_host with the system toolchain

`rules_latex_host` registers a `system` toolchain that wraps the host's
`pdflatex`, `poppler-utils` and `ghostscript`.
That needs those tools installed on every machine that builds, which the
`hermetic` fragment of the coding SOP forbids.
The module also ships a hermetic toolchain, so this was never necessary.


## 2. What the build does now

`MODULE.bazel` declares the hermetic toolchain and registers it from the
root module, which is what makes it win over the `system` toolchain the
module registers for itself.

Three archives make up that toolchain, each pinned by URL and SHA-256.
A TinyTeX distribution, whose binaries locate their own tree with no install
step.
qpdf, which backs the page-count and concatenation contract.
Ghostscript, used only for the outline of a combined PDF.

Nothing is taken from the machine running the build.


## 3. Two packages are vendored, and why

TinyTeX includes neither IEEEtran nor `listings`, and the ruleset says so.
The documented fix is `texlive_archives`, which pins a package archive from
a dated TeX Live snapshot by SHA-256.

That does not work here.
`texlive.info` answers Bazel with different bytes on every request.
Two consecutive fetches produced checksums `33f4b564...` and `10509900...`,
while `curl` returned the same 89368 bytes on three tries.
A content-addressed pin cannot match a moving target, so no value of the
attribute succeeds.

`IEEEtran.cls`, `IEEEtrantools.sty`, `listings.sty`, `lstmisc.sty` and
`listings.cfg` are therefore vendored, under `//third_party/ieeetran` and
`//third_party/listings`, so every vendored byte is tracked in one place.
Each is upstream and unmodified, under the LaTeX Project Public License,
with its copyright header intact.
Each directory holds a `LICENSE` and a `README.md` naming the upstream
source.

They cannot be used from there.
`latex_document` copies a `data` file into the build directory under its
package relative path, and pdflatex searches the build directory rather than
a tree beneath it, so a file from `//third_party` lands at `third_party/...`
and is never found.
`//docs` copies them into its own package with a `genrule` first, which puts
them next to `article.tex`.


## 4. Fonts

The article is set in **Latin Modern**, the Computer Modern variety that
works under pdflatex.

Two things had to be forced.

IEEEtran selects Times on its own.
The first build came out in `ptm` with nothing having asked for it.

Plain `cmr` under `[T1]{fontenc}` is not the fix.
The T1 Computer Modern outlines live in `cm-super`, which is not in the
pinned distribution, so pdflatex falls back to EC bitmaps.
That build produced a PDF with no Type 1 face at all and every glyph a
Type 3 bitmap.
IEEE rejects Type 3, and it renders badly at any zoom.

`lmodern` fixed it.
The same document, with nothing else changed, came out with all 16 faces
Type 1.

Check a finished PDF rather than the source:

```sh
pdffonts bazel-bin/docs/article.pdf | awk 'NR>2 {print $2, $3}' | sort | uniq -c
```

Every line should read `Type 1`.

New Computer Modern in its Book weight is the less spindly Computer Modern,
and it would be the better face at 9pt in two columns.
It needs LuaLaTeX or XeLaTeX: the package requires `fontspec` and
`unicode-math`, and version 8.1.1 ships 41 OpenType files and no Type 1 at
all.
The `latex-pdf-tutorial` skill in the coding SOP states both halves of this.


## 5. Figures are TikZ

Both figures in the article are TikZ pictures.

That needed `pgf`, which the pinned TeX distribution does not include, so it
is vendored under `//third_party/pgf`.
The tree is flattened, because `latex_document` copies a `data` file under
its package relative path and pdflatex searches the build directory rather
than a tree beneath it.
Flattening is safe for this package, and that was checked rather than
assumed: no file in pgf reads another by a path, only by a bare name.
Four files collide across upstream's four trees, all of them `.lua` files
belonging to the graph drawing libraries that only LuaTeX loads, and they
are dropped.
That leaves 282 files and 4.4 MB.

A TikZ picture can be wrong in a way that produces no warning and no error.
The document compiles clean, every reference resolves, no box is overfull,
and an arrowhead is still buried in a box border.
The only way to find that is to render the pages and look at them:

```sh
PDF=bazel-bin/docs/article.pdf
N=$(pdfinfo $PDF | awk '/Pages/{print $2}')
for p in $(seq 1 $N); do
  pdftotext -f $p -l $p $PDF - 2>/dev/null | grep -q '^Fig\.' \
    && echo "figure on page $p"
done
pdftoppm -r 150 -png -f <p> -l <p> $PDF /tmp/fig
```

That pass found two defects that the build reported as success.
The facet boxes centred their item lists, which reads as ragged rather than
as a list.
And Figure 1 drew its distinction with a 3 per cent grey difference, which
is invisible in print, on a figure whose entire message is that
distinction.
Both are fixed: the lists are left aligned, and a worked-out mechanism is
now shaded and solid while one that is only named is unshaded and dashed.
Two channels, so the figure survives being photocopied.

## 6. What is built and released

| Target | Output |
|---|---|
| `//docs:article` | the specification as an IEEE two-column PDF |

`.forgejo/workflows/release.yml` gathers every `*.pdf` and `*.html` under
`bazel-bin/` into `dist/release/`, so the article is attached to the rolling
`nightly` release and to every dated release cut by hand.

`bazel test //...` exits 4 today, because no test target exists yet.
The build workflow treats that one exit code as success and fails on every
other, so a test added later runs without editing the workflow.


## 7. Remaining work

1. **Move the code examples into their own files.**
   A specification whose examples are fenced blocks cannot be checked.
   One whose examples are files can.
   When the parser exists, the test is that every file under `examples/`
   parses, and that every production in the grammar appears in at least one
   file.
   That is what makes the `eng-standards` rule true rather than
   aspirational: every piece of specification must have a test, and that
   test must pass.
2. **Finish the merged specification.**
   `spec/merged-draft.md` holds five written sections and the full section
   skeleton.
   `docs/unification-analysis.md` section 8 states the order to write the
   rest in.
3. **Fold the article and the specification together** once the
   specification is finished, so there is one source rather than two.
4. **Decide the fate of `filmil/workspace/`**, deferred in
   `docs/unification-analysis.md` section 7.
