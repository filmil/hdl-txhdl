# third_party

Files from other projects, kept here so every vendored byte is in one place.

Each subdirectory holds the files, a `LICENSE`, and a `README.md` naming the
upstream source and saying why the files are vendored rather than fetched.
Nothing here is modified: copyright headers are intact and the bytes match
upstream.

| Directory | What | License |
|---|---|---|
| `ieeetran/` | The IEEE journal document class the article is set in | LPPL 1.3 |
| `listings/` | Code listing support the article uses for every example | LPPL |
| `pgf/` | TikZ, which draws every figure in the article | GPL or LPPL 1.3c |

All three are LaTeX packages that the pinned TeX distribution does not
include, and that cannot be fetched reproducibly.
`docs/document-build-plan.md` section 3 states the measurements behind that.

A LaTeX document cannot read these in place.
`rules_latex_host` copies a `data` file into the build directory under its
package relative path, and pdflatex searches the build directory rather than
a tree beneath it, so a file here would land at `third_party/...` and not be
found.
`//docs` copies them into its own package with a `genrule` first.
