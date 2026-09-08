# pgf and TikZ

Upstream: the `pgf` package of TeX Live, from the tlnet snapshot at
`https://texlive.info/tlnet-archive/2026/08/01/tlnet/archive/pgf.tar.xz`
(718196 bytes).

## The tree is flattened, and that is deliberate

Upstream ships four trees: `tex/latex`, `tex/generic`, `tex/plain` and
`tex/context`.
The files here are those trees flattened into one directory.

`rules_latex_host` copies a `data` file into the build directory under its
package relative path, and pdflatex searches the build directory rather than
a tree beneath it.
A nested layout would therefore never be found.
Flattening is safe for this package, and that was checked rather than
assumed: no file in pgf reads another by a path, only by a bare name, so
every `\input` still resolves.

The four files that collide across trees are all `.lua`, and all belong to
the graph drawing libraries, which only LuaTeX loads.
They are dropped.
That leaves 282 files.

Also dropped: the ConTeXt tree, and the documentation.

## License

pgf is distributed under the GNU Public License and the LaTeX Project Public
License, at the user's choice.
`LICENSE` holds LPPL 1.3c, from `https://www.latex-project.org/lppl/`, which
is the choice this project takes.
The per-file copyright headers are intact and state both.

## Why vendored rather than fetched

`texlive.info` answers Bazel with a different byte stream on every request,
so a content-addressed pin cannot match it.
`docs/document-build-plan.md` section 3 has the measurements.
