# Building the specification documents

Status: plan, September 8, 2026.
Author: automated coding assistant, with human supervision.

The specification is written in Markdown, and Bazel renders it to PDF and
HTML with `bazel_ebook`.

An earlier draft of this document planned an IEEEtran article built with
`rules_latex_host`.
That is dropped.
`rules_latex_host` shells out to the host's `pdflatex`, so it could not be
made hermetic without vendoring a TeX distribution, and the article was not
essential.


## 1. Hermeticity: what was wrong and what fixed it

The repository used to pin
`bazel_dep(name = "bazel_ebook", version = "2.0.15")`.
That version depends on `bazel_rules_bid` 1.2.4 and runs pandoc and TeX by
shelling out to `docker`.
On a machine without Docker the build fails, and this was measured rather
than guessed:

```
bazel-out/k8-opt-exec/bin/external/bazel_rules_bid+/build/docker_run:
  line 198: docker: command not found
ERROR: Building PDF for: language-pdf failed
```

`bazel_ebook` **3.0.0**, published September 8, 2026, fixes it.
It drops `bazel_rules_bid` entirely and provisions every tool through Bazel:

| Tool | Source in 3.0.0 |
|---|---|
| `pandoc`, `plantuml` | `@multitool`, pinned in `multitool.lock.json` |
| `pandoc-crossref` | a release tarball pinned by sha256, matched to that pandoc |
| `dot` | the `graphviz` Bazel module, built from source |
| the C compiler that builds it | `hermetic_cc_toolchain`, so the compiler is the same on every machine |
| `pdflatex`, `latex`, `dvipng`, `dvisvgm`, `gs` | a rootfs built by `rules_distroless` from `https://snapshot.ubuntu.com/ubuntu/` |
| `asy`, `drawtiming`, `gladtex`, `ebook-convert`, `rsvg-convert` | the same rootfs |
| fonts | the DejaVu release archive, pinned by sha256 |

The apt packages are pinned in `image/packages.lock.json`, and the archive
is a snapshot service rather than a live mirror, so the same build fetches
the same bytes next year.

One part of that rootfs is worth understanding, because it is the piece that
usually goes wrong.
A rootfs assembled from `.deb` files holds the contents of those files and
nothing else, since no maintainer script has run.
TeX is configured almost entirely by its postinst, so `fmtutil.cnf`,
`updmap.cfg`, and the compiled `.fmt` files are all absent, and `pdflatex`
will not start without them.
`image/texmf.bzl` builds them at build time using the TeX in the rootfs.
Nothing is downloaded for that step and nothing comes from the host.

The toolchain target that ties this together is still named
`docker_toolchain`, which now misleads.
Its `ebook_toolchain` sets only `hermetic_tools` and sets neither `tools`
nor `wrapper`.

### One version skew to work around

`bazel_ebook` 3.0.0 calls `rootfs_binary(env = ...)` in six places in
`image/BUILD.bazel`, and pins `bazel_rootfs` 1.1.1, whose `rootfs_binary`
has no `env` attribute.
Its `//image` package therefore fails to analyze, and so does
`//build/toolchains:default_tools`, which reads `//image:texmf`:

```
ERROR: @@bazel_ebook+//image/BUILD.bazel:121:14: @@bazel_ebook+//image:drawtiming:
  no such attribute 'env' in 'rootfs_binary' rule
```

`bazel_rootfs` 1.2.1 adds the attribute.
Naming it as a direct dependency in this repository's `MODULE.bazel` raises
the resolved version through minimal version selection, and the comment
there says to remove it once `bazel_ebook` pins it itself:

```python
bazel_dep(name = "bazel_rootfs", version = "1.2.1")
```

The real fix belongs upstream, in a `bazel_ebook` 3.0.1 that pins
`bazel_rootfs` 1.2.1 or later.

**Verified end to end.**
`bazelisk build //spec:language-pdf` extracts the rootfs, compiles the
`tex`, `latex` and `pdflatex` formats, and produces a 45 page PDF.
Docker is not involved at any point, and nothing is taken from the machine
running the build.


## 2. What gets built

| Target | Output |
|---|---|
| `//spec:language-pdf` | the specification as a PDF |
| `//spec:language-html` | the specification as a standalone HTML page |

`spec/BUILD.bazel` follows the pattern already used in
`filmil/workspace/BUILD.bazel`: a `markdown_lib` naming the source, an
`ebook_pdf`, and a `pandoc_standalone_html`.
`ebook_pdf` asks for EPUB metadata even when only a PDF is wanted, so a
`genrule` supplies an empty XML file, and `spec/title.yaml` supplies the
title and the authors.

Before this change `spec/language.md` had no Bazel target at all.
The four smaller files under `filmil/workspace/` were built and published,
and the 2241 line specification was not.

The release workflow publishes `bazel-bin/filmil` to
`filmil/hdlfactory.com.template` under `static/txhdl/filmil`.
Once the specification builds, that path should publish `bazel-bin/spec` as
well, or instead.
Issue #1 covers the workflow work, including the fact that neither workflow
currently runs.


## 3. What the first successful render found

The PDF is 45 pages on US letter, and the LaTeX run reports **zero overfull
boxes**, which was the failure mode worth worrying about with 20 sections of
wide code blocks.

It reports **20 undefined hyper references**, one per entry in the
hand-written Table of Contents at `spec/language.md:9`.
Anchors such as `#1-design-philosophy` resolve in a Markdown viewer and in
HTML, and resolve to nothing in the PDF.

The fix is to delete the hand-written Table of Contents and let pandoc
generate one.
That also removes a list that has to be edited by hand every time a section
is added, renamed, or renumbered, which is exactly the kind of list that
goes stale.

One further warning comes from `bazel_ebook` itself, not from this
repository: `Deprecated: --mathml. Use --math-method=mathml instead.`


## 4. Keep the examples out of the prose

This part survives from the earlier draft, because it has nothing to do with
the output format.

A specification whose code examples are fenced blocks inside the prose
cannot be checked.
A specification whose examples are files in a directory can.

```
   examples/
     mac_unit.tx          <-- the only copy of this code
     wishbone.tx
     dma.tx
        |
        |  pandoc include filter
        v
   spec/language.md
        |
        v
   PDF and HTML
```

`fragments/eng-standards.md` in the coding SOP requires that every piece of
specification has a test, and that the test passes.
Prose cannot satisfy that.
Files can: when the parser exists, the test is that every file under
`examples/` parses, and that every production in the grammar appears in at
least one file.

`bazel_ebook` already depends on `pandoc-include` and the `include-files`
pandoc extension, so the mechanism is present and needs no new dependency.

Do this while rewriting the specification, not after.
Retrofitting it means editing the document by hand once.


## 5. Work items

1. **Move to `bazel_ebook` 3.0.0.**
   Done on this branch, with the `bazel_rootfs` 1.2.1 workaround from
   section 1.
2. **Get `bazel_ebook` 3.0.1 released** with `bazel_rootfs` 1.2.1 or later
   pinned, then delete the workaround.
3. **Add `spec/BUILD.bazel` and `spec/title.yaml`.**
   Done on this branch.
4. **Raise `.bazelversion` from `9.0.1` to `9.2.0`.**
   Done on this branch.
   The `bazel` fragment of the coding SOP requires `9.2.0` or later, because
   `9.1.0` and earlier crash with a `NullPointerException` when fetching a
   repository named by a `file://` URL.
   `ProgressInputStream.reportProgress` calls `String.equals` on
   `URI.getHost()`, which returns `null` for such a URL.
   The workflows pin `9.0.1` too, and issue #1 covers those.
5. **Replace the hand-written Table of Contents** with a generated one.
   Section 3.
6. **Create `examples/` and move code out of the prose into it.**
   Do this as part of writing the merged specification.
7. **Publish `bazel-bin/spec` from the release workflow.**
   Part of issue #1.
8. **Retire the `filmil/workspace` document targets** once the decision
   deferred in `unification-analysis.md` section 7 is made.


## 6. Things that will bite

**The first build fetches a lot.**
944 actions and just under 10 minutes on a warm network and a cold Bazel
cache.
A rootfs, a Zig toolchain, graphviz from source, calibre, and a TeX
installation.
That is the cost of a build that asks the machine for nothing, and it is
paid once per machine.
Cache it in CI, which issue #1 already provides for.

**`ebook_pdf` needs `//:empty_md` in its deps.**
Every existing target in the repository lists it, and the pattern gets
copied rather than understood.
Keep listing it.

**Do not commit the PDF.**
`.gitignore` excludes `bazel-*`, and the artifact is delivered from
`bazel-bin/spec/`.
