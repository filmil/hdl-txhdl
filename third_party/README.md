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

`docs/document-build-plan.md` section 3 states the measurements behind
vendoring them.

A LaTeX document cannot read these in place.
`rules_latex_host` copies a `data` file into the build directory under its
package relative path, and pdflatex searches the build directory rather than
a tree beneath it, so a file here would land at `third_party/...` and not be
found.
`//docs` copies them into its own package with a `genrule` first.

## Pins rather than files

The other directories hold no vendored bytes at all.
Most hold lock files: a row per file or per package, with a checksum and
where to fetch it from, which the build reads and fetches.
That is the better arrangement whenever what is needed is large, is
published somewhere stable, and is not modified here.

| Directory | What it pins |
|---|---|
| `debs/` | `repo.bzl`, the rule that reads such a lock file and unpacks what it names |
| `yosys/` | Yosys and the ten libraries it needs, from Debian trixie |
| `openroad/` | OpenROAD, and the closure of a hundred and thirty one packages it links against, from Debian bullseye |
| `nangate45/` | The six files of the open 45 nm standard cell library the ASIC flow reads |
| `poppler/` | Poppler's utilities and their libraries, from Debian trixie, for the documents' edge check |
| `openocd/` | OpenOCD 0.12.0 and its fifteen packages, the C library among them, from Debian trixie, run through the tree's own loader, for the debug transport's harness |
| `z3/` | z3 and its libraries, from Debian trixie, for the formal flow's bounded search |
| `sby/` | SymbiYosys, fetched from its release tag by checksum, and `requirements.txt`, its Python pins |
| `zephyr/` | The build file Zephyr's fetched archive is given, and `requirements.txt`, the Python its build imports |
| `riscv_gcc/` | The build file the fetched RISC-V toolchain is given |
| `llvm/` | The extension that makes the hermetic LLVM C and C++ toolchain from `toolchains_llvm`, over the sysroot below (issue 782) |
| `sysroot/` | The Debian bullseye C library and GCC runtime the LLVM toolchain links against, as a lock file (issue 782) |
| `gdb/` | gdb-multiarch and its libraries, from Debian trixie, for the debug transport's harness (issue 872) |

`rules_multitool_root_hubs.patch` beside them is a patch to a fetched
module, upstream's own fix, applied until it reaches the registry.

`//tools/debclosure` writes the Debian lock files; run it again to
move a pin.
Every one of those rows carries two URLs, the live archive and
`snapshot.debian.org`, so a pin that the archive has forgotten still
fetches.
