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

`uberddr3/` is the exception: it holds no upstream files, only the build
file of a repository `MODULE.bazel` fetches, and its `README.md` says why.
`docs/document-build-plan.md` section 3 states the measurements behind that.

A LaTeX document cannot read these in place.
`rules_latex_host` copies a `data` file into the build directory under its
package relative path, and pdflatex searches the build directory rather than
a tree beneath it, so a file here would land at `third_party/...` and not be
found.
`//docs` copies them into its own package with a `genrule` first.

## Pins rather than files

Four more directories hold no vendored bytes at all.
They hold lock files: a row per file or per package, with a checksum and
where to fetch it from, which the build reads and fetches.
That is the better arrangement whenever what is needed is large, is
published somewhere stable, and is not modified here.

| Directory | What it pins |
|---|---|
| `debs/` | `repo.bzl`, the rule that reads such a lock file and unpacks what it names |
| `yosys/` | Yosys and the ten libraries it needs, from Debian trixie |
| `openroad/` | OpenROAD, and the closure of a hundred and thirty one packages it links against, from Debian bullseye |
| `nangate45/` | The six files of the open 45 nm standard cell library the ASIC flow reads |

`//tools/debclosure` writes the two Debian lock files; run it again to
move a pin.
Every one of those rows carries two URLs, the live archive and
`snapshot.debian.org`, so a pin that the archive has forgotten still
fetches.
